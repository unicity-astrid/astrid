//! Plugin hot-reload watcher and related helpers.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use astrid_core::{InboundMessage, SessionId};
use astrid_mcp::McpClient;
use astrid_plugins::manifest::PluginEntryPoint;
use astrid_plugins::{PluginContext, PluginId, PluginRegistry, PluginState, WasmPluginLoader};
use astrid_storage::{KvStore, ScopedKvStore};
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::monitoring::AbortOnDrop;
use super::{DaemonServer, SessionHandle};
use crate::rpc::DaemonEvent;

/// Shared context passed to `handle_watcher_reload` to avoid exceeding the
/// clippy `too_many_arguments` limit.
pub(super) struct WatcherReloadContext {
    pub(super) plugin_registry: Arc<RwLock<PluginRegistry>>,
    pub(super) workspace_kv: Arc<dyn KvStore>,
    pub(super) sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    pub(super) mcp_client: McpClient,
    pub(super) workspace_root: PathBuf,
    pub(super) user_unloaded: Arc<RwLock<HashSet<PluginId>>>,
    pub(super) wasm_loader: Arc<WasmPluginLoader>,
    /// Central inbound channel sender — cloned for each hot-reloaded connector plugin.
    pub(super) inbound_tx: mpsc::Sender<InboundMessage>,
}

impl DaemonServer {
    /// Spawn the plugin hot-reload watcher.
    ///
    /// Watches `~/.astrid/plugins/` and `.astrid/plugins/` (workspace) for
    /// filesystem changes. When a plugin's files change (debounced, blake3
    /// deduplicated), the affected plugin is unloaded, re-discovered, and
    /// reloaded automatically.
    ///
    /// Returns `None` if no plugin directories exist yet.
    #[must_use]
    pub fn spawn_plugin_watcher(&self) -> Option<tokio::task::JoinHandle<()>> {
        use astrid_plugins::watcher::{PluginWatcher, WatchEvent, WatcherConfig};

        let mut watch_paths: Vec<PathBuf> = Vec::new();

        // User-level plugins.
        let user_plugins = self.home.plugins_dir();
        if user_plugins.exists() {
            watch_paths.push(
                user_plugins
                    .canonicalize()
                    .unwrap_or_else(|_| user_plugins.clone()),
            );
        }

        // Workspace-level plugins.
        let ws_plugins = self.workspace_root.join(".astrid/plugins");
        if ws_plugins.exists() {
            watch_paths.push(
                ws_plugins
                    .canonicalize()
                    .unwrap_or_else(|_| ws_plugins.clone()),
            );
        }

        if watch_paths.is_empty() {
            info!("No plugin directories to watch — plugin watcher not started");
            return None;
        }

        let config = WatcherConfig {
            watch_paths,
            ..Default::default()
        };

        let (watcher, mut events) = match PluginWatcher::new(config) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "Failed to create plugin file watcher");
                return None;
            },
        };

        // Log messages are emitted by PluginWatcher::run() when it starts
        // watching each directory -- no need to log here too.

        let reload_ctx = WatcherReloadContext {
            plugin_registry: Arc::clone(&self.plugin_registry),
            workspace_kv: Arc::clone(&self.workspace_kv),
            sessions: Arc::clone(&self.sessions),
            mcp_client: self.mcp_client.clone(),
            workspace_root: self.workspace_root.clone(),
            user_unloaded: Arc::clone(&self.user_unloaded_plugins),
            wasm_loader: Arc::clone(&self.wasm_loader),
            inbound_tx: self.inbound_tx.clone(),
        };

        let handle = tokio::spawn(async move {
            // Spawn the watcher event loop in the background.
            let watcher_task = tokio::spawn(async move { watcher.run().await });
            // Guard ensures the inner task is aborted when this outer task
            // is cancelled (e.g. during daemon shutdown via handle.abort()).
            // Without this, JoinHandle::drop does NOT cancel the inner task.
            let _guard = AbortOnDrop(watcher_task);

            while let Some(event) = events.recv().await {
                match event {
                    WatchEvent::PluginChanged { plugin_dir, .. } => {
                        info!(dir = %plugin_dir.display(), "Plugin change detected, reloading");
                        Self::handle_watcher_reload(&plugin_dir, &reload_ctx).await;
                    },
                    WatchEvent::Error(msg) => {
                        warn!(error = %msg, "Plugin watcher error");
                    },
                }
            }
            // _guard dropped here -> inner watcher task is aborted.
        });

        Some(handle)
    }

    /// Handle a single plugin reload triggered by the file watcher.
    ///
    /// Discovers the manifest in the changed directory, unloads the old plugin
    /// if loaded, re-registers it, loads it with a fresh context, and broadcasts
    /// the result to all connected sessions.
    #[allow(clippy::too_many_lines)]
    async fn handle_watcher_reload(plugin_dir: &std::path::Path, ctx: &WatcherReloadContext) {
        let WatcherReloadContext {
            plugin_registry,
            workspace_kv,
            sessions,
            mcp_client,
            workspace_root,
            user_unloaded,
            wasm_loader,
            inbound_tx,
        } = ctx;
        // Try to load the manifest. Compiled plugins have plugin.toml;
        // uncompiled OpenClaw plugins only have openclaw.plugin.json and
        // need to be compiled first (handled by `astrid plugin install`).
        let manifest_path = plugin_dir.join("plugin.toml");
        let mut manifest = match astrid_plugins::load_manifest(&manifest_path) {
            Ok(m) => m,
            Err(_) if plugin_dir.join("openclaw.plugin.json").exists() => {
                debug!(
                    dir = %plugin_dir.display(),
                    "OpenClaw plugin changed but has no compiled plugin.toml — \
                     run `astrid plugin install` to compile"
                );
                return;
            },
            Err(e) => {
                warn!(dir = %plugin_dir.display(), error = %e, "No valid manifest in changed plugin dir");
                return;
            },
        };

        let plugin_id = manifest.id.clone();
        let plugin_id_str = plugin_id.as_str().to_string();

        // Skip plugins the user explicitly unloaded via RPC.
        if user_unloaded.read().await.contains(&plugin_id) {
            debug!(plugin = %plugin_id, "Skipping watcher reload — plugin was user-unloaded");
            return;
        }

        // Resolve relative WASM paths to absolute (same as initial discovery).
        if let PluginEntryPoint::Wasm { ref mut path, .. } = manifest.entry_point
            && path.is_relative()
        {
            *path = plugin_dir.join(&*path);
        }

        // Create the new plugin instance.
        let new_plugin = match Self::create_plugin_from_manifest(
            &manifest,
            mcp_client,
            wasm_loader,
            Some(plugin_dir.to_path_buf()),
        ) {
            Ok(p) => p,
            Err(e) => {
                warn!(plugin = %plugin_id, error = %e, "Failed to create plugin on reload");
                Self::broadcast_event(
                    sessions,
                    DaemonEvent::PluginFailed {
                        id: plugin_id_str,
                        error: e,
                    },
                )
                .await;
                return;
            },
        };

        // Unload -> unregister -> re-register -> load.
        let (was_loaded, load_result) = Self::swap_and_load_plugin(
            &plugin_id,
            new_plugin,
            plugin_registry,
            workspace_kv,
            workspace_root,
        )
        .await;

        // Broadcast PluginUnloaded if it was previously loaded.
        if was_loaded {
            Self::broadcast_event(
                sessions,
                DaemonEvent::PluginUnloaded {
                    id: plugin_id_str.clone(),
                    name: manifest.name.clone(),
                },
            )
            .await;
        }

        match load_result {
            Ok(()) => {
                info!(plugin = %plugin_id, "Hot-reloaded plugin");

                // Wire inbound receiver if the reloaded plugin has connector capability.
                // Takes a brief write lock to call take_inbound_rx() through the trait.
                {
                    let rx = {
                        let mut reg = plugin_registry.write().await;
                        reg.get_mut(&plugin_id).and_then(|p| p.take_inbound_rx())
                    };
                    if let Some(rx) = rx {
                        let tx = inbound_tx.clone();
                        let pid = plugin_id_str.clone();
                        tokio::spawn(async move {
                            super::inbound_router::forward_inbound(pid, rx, tx).await;
                        });
                    }
                }

                Self::broadcast_event(
                    sessions,
                    DaemonEvent::PluginLoaded {
                        id: plugin_id_str,
                        name: manifest.name.clone(),
                    },
                )
                .await;
            },
            Err(e) => {
                warn!(plugin = %plugin_id, error = %e, "Failed to reload plugin");
                Self::broadcast_event(
                    sessions,
                    DaemonEvent::PluginFailed {
                        id: plugin_id_str,
                        error: e,
                    },
                )
                .await;
            },
        }
    }

    /// Create a plugin instance from a manifest.
    pub(super) fn create_plugin_from_manifest(
        manifest: &astrid_plugins::PluginManifest,
        mcp_client: &McpClient,
        wasm_loader: &WasmPluginLoader,
        plugin_dir: Option<PathBuf>,
    ) -> Result<Box<dyn astrid_plugins::Plugin>, String> {
        match &manifest.entry_point {
            PluginEntryPoint::Wasm { .. } => {
                Ok(Box::new(wasm_loader.create_plugin(manifest.clone())))
            },
            PluginEntryPoint::Mcp { .. } => astrid_plugins::create_plugin(
                manifest.clone(),
                Some(mcp_client.clone()),
                plugin_dir,
            )
            .map_err(|e| e.to_string()),
        }
    }

    /// Swap a plugin in the registry: unload old, unregister, register new, load.
    ///
    /// Returns `(was_previously_loaded, Result)` so callers can broadcast
    /// the appropriate events.
    pub(super) async fn swap_and_load_plugin(
        plugin_id: &PluginId,
        new_plugin: Box<dyn astrid_plugins::Plugin>,
        plugin_registry: &Arc<RwLock<PluginRegistry>>,
        workspace_kv: &Arc<dyn KvStore>,
        workspace_root: &std::path::Path,
    ) -> (bool, Result<(), String>) {
        let mut registry = plugin_registry.write().await;

        // Unload existing plugin (best-effort). Track if it was loaded.
        let was_loaded = if let Some(existing) = registry.get_mut(plugin_id) {
            let loaded = existing.state() == PluginState::Ready;
            if loaded && let Err(e) = existing.unload().await {
                warn!(plugin = %plugin_id, error = %e, "Error unloading plugin before reload");
            }
            loaded
        } else {
            false
        };

        // Remove and re-register to pick up new manifest/code.
        let _ = registry.unregister(plugin_id);
        if let Err(e) = registry.register(new_plugin) {
            return (was_loaded, Err(e.to_string()));
        }

        // Load the freshly registered plugin.
        let Some(plugin) = registry.get_mut(plugin_id) else {
            return (
                was_loaded,
                Err("plugin disappeared after register".to_string()),
            );
        };
        let kv = match ScopedKvStore::new(Arc::clone(workspace_kv), format!("plugin:{plugin_id}")) {
            Ok(kv) => kv,
            Err(e) => return (was_loaded, Err(e.to_string())),
        };
        let config = plugin.manifest().config.clone();
        let ctx = PluginContext::new(workspace_root.to_path_buf(), kv, config);
        let result = plugin.load(&ctx).await.map_err(|e| e.to_string());
        (was_loaded, result)
    }

    /// Broadcast an event to all connected sessions.
    pub(super) async fn broadcast_event(
        sessions: &Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
        event: DaemonEvent,
    ) {
        let map = sessions.read().await;
        for handle in map.values() {
            let _ = handle.event_tx.send(event.clone());
        }
    }
}
