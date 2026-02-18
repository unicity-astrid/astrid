//! Plugin management RPC method implementations.

use std::sync::Arc;

use astrid_plugins::{PluginContext, PluginId, PluginState};
use astrid_storage::ScopedKvStore;
use jsonrpsee::types::ErrorObjectOwned;

use super::RpcImpl;
use crate::rpc::{DaemonEvent, PluginInfo, error_codes};

impl RpcImpl {
    pub(super) async fn list_plugins_impl(&self) -> Result<Vec<PluginInfo>, ErrorObjectOwned> {
        let registry = self.plugin_registry.read().await;
        let mut infos = Vec::new();
        for id in registry.list() {
            if let Some(plugin) = registry.get(id) {
                let (state_str, error) = match plugin.state() {
                    PluginState::Unloaded => ("unloaded".to_string(), None),
                    PluginState::Loading => ("loading".to_string(), None),
                    PluginState::Ready => ("ready".to_string(), None),
                    PluginState::Failed(msg) => ("failed".to_string(), Some(msg)),
                    PluginState::Unloading => ("unloading".to_string(), None),
                };
                let manifest = plugin.manifest();
                infos.push(PluginInfo {
                    id: id.as_str().to_string(),
                    name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    state: state_str,
                    tool_count: plugin.tools().len(),
                    description: manifest.description.clone(),
                    error,
                });
            }
        }
        Ok(infos)
    }

    pub(super) async fn load_plugin_impl(
        &self,
        plugin_id: String,
    ) -> Result<PluginInfo, ErrorObjectOwned> {
        let pid = PluginId::new(&plugin_id).map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("Invalid plugin id: {e}"),
                None::<()>,
            )
        })?;

        // Clear user-unloaded flag -- user explicitly wants this plugin loaded.
        self.user_unloaded_plugins.write().await.remove(&pid);

        // Take the plugin out of the registry so we can load it without
        // holding the write lock (MCP plugins spawn subprocesses + handshake).
        let mut plugin = {
            let mut registry = self.plugin_registry.write().await;
            registry.unregister(&pid).map_err(|_| {
                ErrorObjectOwned::owned(
                    error_codes::PLUGIN_NOT_FOUND,
                    format!("Plugin not found: {plugin_id}"),
                    None::<()>,
                )
            })?
        };
        // Write lock released -- other registry operations are unblocked.

        let kv = match ScopedKvStore::new(
            Arc::clone(&self.workspace_kv),
            format!("plugin:{plugin_id}"),
        ) {
            Ok(kv) => kv,
            Err(e) => {
                // Put the plugin back before returning the error.
                let mut registry = self.plugin_registry.write().await;
                let _ = registry.register(plugin);
                return Err(ErrorObjectOwned::owned(
                    error_codes::INTERNAL_ERROR,
                    format!("Failed to create plugin KV scope: {e}"),
                    None::<()>,
                ));
            },
        };

        let config = plugin.manifest().config.clone();
        let ctx = PluginContext::new(self.workspace_root.clone(), kv, config);

        // Expensive async load happens outside the lock.
        let load_result = plugin.load(&ctx).await;
        let manifest = plugin.manifest();
        let name = manifest.name.clone();

        let (state_str, error, event) = match load_result {
            Ok(()) => (
                "ready".to_string(),
                None,
                DaemonEvent::PluginLoaded {
                    id: plugin_id.clone(),
                    name: name.clone(),
                },
            ),
            Err(e) => {
                let err_msg = e.to_string();
                (
                    "failed".to_string(),
                    Some(err_msg.clone()),
                    DaemonEvent::PluginFailed {
                        id: plugin_id.clone(),
                        error: err_msg,
                    },
                )
            },
        };

        let tool_count = plugin.tools().len();
        let version = manifest.version.clone();
        let description = manifest.description.clone();

        // Brief write lock to put the plugin back.
        {
            let mut registry = self.plugin_registry.write().await;
            let _ = registry.register(plugin);
        }

        let info = PluginInfo {
            id: plugin_id,
            name,
            version,
            state: state_str,
            tool_count,
            description,
            error,
        };

        self.broadcast_to_all_sessions(event).await;

        if info.state == "failed" {
            return Err(ErrorObjectOwned::owned(
                error_codes::PLUGIN_ERROR,
                format!(
                    "Plugin load failed: {}",
                    info.error.as_deref().unwrap_or("unknown")
                ),
                None::<()>,
            ));
        }

        Ok(info)
    }

    pub(super) async fn unload_plugin_impl(
        &self,
        plugin_id: String,
    ) -> Result<(), ErrorObjectOwned> {
        let pid = PluginId::new(&plugin_id).map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("Invalid plugin id: {e}"),
                None::<()>,
            )
        })?;

        // Take the plugin out so we can unload without holding the lock
        // (MCP plugins may need to shut down child processes).
        let mut plugin = {
            let mut registry = self.plugin_registry.write().await;
            registry.unregister(&pid).map_err(|_| {
                ErrorObjectOwned::owned(
                    error_codes::PLUGIN_NOT_FOUND,
                    format!("Plugin not found: {plugin_id}"),
                    None::<()>,
                )
            })?
        };

        let name = plugin.manifest().name.clone();

        // Unload outside the lock.
        let unload_result = plugin.unload().await;

        // Always put the plugin back (brief write lock).
        {
            let mut registry = self.plugin_registry.write().await;
            let _ = registry.register(plugin);
        }

        // Now check the result after re-registering.
        unload_result.map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::PLUGIN_ERROR,
                format!("Failed to unload plugin: {e}"),
                None::<()>,
            )
        })?;

        // Mark as user-unloaded so the watcher doesn't re-load it.
        self.user_unloaded_plugins.write().await.insert(pid);

        let event = DaemonEvent::PluginUnloaded {
            id: plugin_id,
            name,
        };

        self.broadcast_to_all_sessions(event).await;

        Ok(())
    }
}
