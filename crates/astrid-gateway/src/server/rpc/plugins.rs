//! Plugin management RPC method implementations.

use std::sync::Arc;

use astrid_capsule::capsule::{CapsuleId, CapsuleState};
use astrid_capsule::context::CapsuleContext;
use astrid_storage::ScopedKvStore;
use jsonrpsee::types::ErrorObjectOwned;

use super::RpcImpl;
use crate::rpc::{CapsuleInfo, DaemonEvent, error_codes};

impl RpcImpl {
    pub(super) async fn list_capsules_impl(&self) -> Result<Vec<CapsuleInfo>, ErrorObjectOwned> {
        let registry: tokio::sync::RwLockReadGuard<'_, astrid_capsule::registry::CapsuleRegistry> =
            self.plugins.read().await;
        let mut infos = Vec::new();
        for id in registry.list() {
            if let Some(plugin) = registry.get(id) {
                let (state_str, error) = match plugin.state() {
                    CapsuleState::Unloaded => ("unloaded".to_string(), None),
                    CapsuleState::Loading => ("loading".to_string(), None),
                    CapsuleState::Ready => ("ready".to_string(), None),
                    CapsuleState::Failed(msg) => ("failed".to_string(), Some(msg)),
                    CapsuleState::Unloading => ("unloading".to_string(), None),
                };
                let manifest = plugin.manifest();
                infos.push(CapsuleInfo {
                    id: id.as_str().to_string(),
                    name: manifest.package.name.clone(),
                    version: manifest.package.version.clone(),
                    state: state_str,
                    tool_count: plugin.tools().len(),
                    description: manifest.package.description.clone(),
                    error,
                });
            }
        }
        Ok(infos)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) async fn load_capsule_impl(
        &self,
        plugin_id: String,
    ) -> Result<CapsuleInfo, ErrorObjectOwned> {
        let pid = CapsuleId::new(&plugin_id).map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("Invalid plugin id: {e}"),
                None::<()>,
            )
        })?;

        // Clear user-unloaded flag -- user explicitly wants this plugin loaded.
        self.user_unloaded_capsules.write().await.remove(&pid);

        // Take the plugin out of the registry so we can load it without
        // holding the write lock (MCP plugins spawn subprocesses + handshake).
        let mut plugin = {
            let mut registry = self.plugins.write().await;
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
            format!("capsule:{plugin_id}"),
        ) {
            Ok(kv) => kv,
            Err(e) => {
                // Put the plugin back before returning the error.
                let mut registry = self.plugins.write().await;
                let _ = registry.register(plugin);
                return Err(ErrorObjectOwned::owned(
                    error_codes::INTERNAL_ERROR,
                    format!("Failed to create plugin KV scope: {e}"),
                    None::<()>,
                ));
            },
        };

        let ctx = CapsuleContext::new(self.workspace_root.clone(), kv, Arc::clone(&self.event_bus));

        // Expensive async load happens outside the lock.
        let load_result: astrid_capsule::error::CapsuleResult<()> = plugin.load(&ctx).await;
        let manifest = plugin.manifest();
        let name = manifest.package.name.clone();

        let load_succeeded = load_result.is_ok();
        let (state_str, error, event) = match load_result {
            Ok(()) => (
                "ready".to_string(),
                None,
                DaemonEvent::CapsuleLoaded {
                    id: plugin_id.clone(),
                    name: name.clone(),
                },
            ),
            Err(e) => {
                let err_msg = e.to_string();
                (
                    "failed".to_string(),
                    Some(err_msg.clone()),
                    DaemonEvent::CapsuleFailed {
                        id: plugin_id.clone(),
                        error: err_msg,
                    },
                )
            },
        };

        let tool_count = plugin.tools().len();
        let version = manifest.package.version.clone();
        let description = manifest.package.description.clone();

        // Take the inbound receiver before re-registering, while we have
        // exclusive ownership of the plugin. Mirrors the auto-load and
        // hot-reload watcher paths. Only meaningful on successful load.
        let inbound_rx = if load_succeeded {
            plugin.take_inbound_rx()
        } else {
            None
        };

        // Brief write lock to put the plugin back.
        {
            let mut registry = self.plugins.write().await;
            let _ = registry.register(plugin);
        }

        // Spawn the fan-in forwarder after releasing the registry lock.
        // JoinHandle intentionally discarded: self-terminating (see inbound_router.rs).
        if let Some(rx) = inbound_rx {
            let tx = self.inbound_tx.clone();
            let pid = plugin_id.clone();
            tokio::spawn(async move {
                crate::server::inbound_router::forward_inbound(pid, rx, tx).await;
            });
        }

        let info = CapsuleInfo {
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

    pub(super) async fn unload_capsule_impl(
        &self,
        plugin_id: String,
    ) -> Result<(), ErrorObjectOwned> {
        let pid = CapsuleId::new(&plugin_id).map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("Invalid plugin id: {e}"),
                None::<()>,
            )
        })?;

        // Take the plugin out so we can unload without holding the lock
        // (MCP plugins may need to shut down child processes).
        let mut plugin = {
            let mut registry = self.plugins.write().await;
            registry.unregister(&pid).map_err(|_| {
                ErrorObjectOwned::owned(
                    error_codes::PLUGIN_NOT_FOUND,
                    format!("Plugin not found: {plugin_id}"),
                    None::<()>,
                )
            })?
        };

        let name = plugin.manifest().package.name.clone();

        // Unload outside the lock.
        let unload_result: astrid_capsule::error::CapsuleResult<()> = plugin.unload().await;

        // Always put the plugin back (brief write lock).
        {
            let mut registry = self.plugins.write().await;
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
        self.user_unloaded_capsules.write().await.insert(pid);

        let event = DaemonEvent::CapsuleUnloaded {
            id: plugin_id,
            name,
        };

        self.broadcast_to_all_sessions(event).await;

        Ok(())
    }
}
