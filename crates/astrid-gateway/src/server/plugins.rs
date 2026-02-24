//! Plugin hot-reload watcher and related helpers.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use astrid_capsule::capsule::{Capsule, CapsuleId, CapsuleState};
use astrid_capsule::context::CapsuleContext;
use astrid_capsule::registry::CapsuleRegistry;
use astrid_core::SessionId;
use astrid_storage::{KvStore, ScopedKvStore};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::monitoring::AbortOnDrop;
use super::{DaemonServer, SessionHandle};
use crate::rpc::DaemonEvent;

/// Shared context passed to `handle_watcher_reload` to avoid exceeding the
/// clippy `too_many_arguments` limit.
pub(super) struct WatcherReloadContext {
    pub(super) plugins: Arc<RwLock<CapsuleRegistry>>,
    pub(super) workspace_kv: Arc<dyn KvStore>,
    pub(super) sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    pub(super) workspace_root: PathBuf,
    pub(super) user_unloaded: Arc<RwLock<HashSet<CapsuleId>>>,
    pub(super) inbound_tx: tokio::sync::mpsc::Sender<astrid_core::InboundMessage>,
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
        use astrid_capsule::watcher::{CapsuleWatcher, WatchEvent, WatcherConfig};

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

        let (watcher, mut events) = match CapsuleWatcher::new(config) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "Failed to create plugin file watcher");
                return None;
            },
        };

        // Log messages are emitted by PluginWatcher::run() when it starts
        // watching each directory -- no need to log here too.

        let reload_ctx = WatcherReloadContext {
            plugins: Arc::clone(&self.plugins),
            workspace_kv: Arc::clone(&self.workspace_kv),
            sessions: Arc::clone(&self.sessions),
            workspace_root: self.workspace_root.clone(),
            user_unloaded: Arc::clone(&self.user_unloaded_capsules),
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
                    WatchEvent::CapsuleChanged { capsule_dir, .. } => {
                        info!(dir = %capsule_dir.display(), "Capsule change detected, reloading");
                        Self::handle_watcher_reload(&capsule_dir, &reload_ctx).await;
                    },
                    WatchEvent::Error(msg) => {
                        warn!(error = %msg, "Capsule watcher error");
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
            plugins,
            workspace_kv,
            sessions,
            workspace_root,
            user_unloaded,
            inbound_tx,
        } = ctx;
        // Try to load the manifest. Compiled plugins have Capsule.toml;
        // uncompiled OpenClaw plugins only have openclaw.plugin.json and
        // need to be compiled first (handled by `astrid capsule install`).
        let manifest_path = plugin_dir.join("Capsule.toml");
        let mut manifest = match astrid_capsule::discovery::load_manifest(&manifest_path) {
            Ok(m) => m,
            Err(_) if plugin_dir.join("openclaw.plugin.json").exists() => {
                debug!(
                    dir = %plugin_dir.display(),
                    "OpenClaw plugin changed but has no compiled Capsule.toml — \
                     run `astrid capsule install` to compile"
                );
                return;
            },
            Err(e) => {
                warn!(dir = %plugin_dir.display(), error = %e, "No valid manifest in changed plugin dir");
                return;
            },
        };

        let plugin_id = CapsuleId::from_static(&manifest.package.name);
        let plugin_id_str = plugin_id.as_str().to_string();

        // Skip plugins the user explicitly unloaded via RPC.
        if user_unloaded.read().await.contains(&plugin_id) {
            debug!(plugin = %plugin_id, "Skipping watcher reload — plugin was user-unloaded");
            return;
        }

        // Resolve relative WASM paths to absolute (same as initial discovery).
        if let Some(component) = &mut manifest.component
            && component.entrypoint.is_relative()
        {
            component.entrypoint = plugin_dir.join(&component.entrypoint);
        }

        // Create the new plugin instance.
        let loader = astrid_capsule::loader::CapsuleLoader::new();
        let new_plugin = match loader.create_capsule(manifest.clone(), plugin_dir.to_path_buf()) {
            Ok(p) => p,
            Err(e) => {
                warn!(plugin = %plugin_id, error = %e, "Failed to create capsule on reload");
                Self::broadcast_event(
                    sessions,
                    DaemonEvent::PluginFailed {
                        id: plugin_id_str,
                        error: e.to_string(),
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
            plugins,
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
                    name: manifest.package.name.clone(),
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
                        let mut reg = plugins.write().await;
                        reg.get_mut(&plugin_id).and_then(|p| p.take_inbound_rx())
                    };

                    if let Some(rx) = rx {
                        let tx = inbound_tx.clone();
                        let pid = plugin_id.to_string();
                        tokio::spawn(async move {
                            crate::server::inbound_router::forward_inbound(pid, rx, tx).await;
                        });
                    }
                }

                Self::broadcast_event(
                    sessions,
                    DaemonEvent::PluginLoaded {
                        id: plugin_id_str,
                        name: manifest.package.name.clone(),
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

    /// Swap a plugin in the registry: unload old, unregister, register new, load.
    ///
    /// Returns `(was_previously_loaded, Result)` so callers can broadcast
    /// the appropriate events.
    pub(super) async fn swap_and_load_plugin(
        plugin_id: &CapsuleId,
        mut new_plugin: Box<dyn Capsule>,
        plugins: &Arc<RwLock<CapsuleRegistry>>,
        workspace_kv: &Arc<dyn KvStore>,
        workspace_root: &std::path::Path,
    ) -> (bool, Result<(), String>) {
        // Step 1: Create the execution context for the new plugin first.
        // If this fails, we return early and leave the old plugin running.
        let kv = match ScopedKvStore::new(Arc::clone(workspace_kv), format!("plugin:{plugin_id}")) {
            Ok(kv) => kv,
            Err(e) => return (false, Err(e.to_string())),
        };
        let ctx = CapsuleContext::new(workspace_root.to_path_buf(), kv);

        // Step 2: Remove existing plugin under a brief lock.
        let (was_loaded, mut existing_plugin) = {
            let mut registry = plugins.write().await;
            let existing = registry.unregister(plugin_id).ok();
            let was_loaded = existing
                .as_ref()
                .is_some_and(|p| p.state() == CapsuleState::Ready);
            (was_loaded, existing)
        };

        // Unload existing plugin outside the lock.
        if let Some(existing) = &mut existing_plugin
            && was_loaded
            && let Err(e) = existing.unload().await
        {
            warn!(plugin = %plugin_id, error = %e, "Error unloading plugin before reload");
        }

        // Step 3: Load the new plugin outside the lock.
        let load_result = new_plugin.load(&ctx).await.map_err(|e| e.to_string());

        // Step 4: Re-register the newly loaded plugin under a brief lock.
        let mut registry = plugins.write().await;
        if let Err(e) = registry.register(new_plugin) {
            return (was_loaded, Err(e.to_string()));
        }

        (was_loaded, load_result)
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
