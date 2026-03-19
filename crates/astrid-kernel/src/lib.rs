#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(clippy::module_name_repetitions)]

//! Astrid Kernel - The core execution engine and IPC router.
//!
//! The Kernel is a pure, decentralized WASM runner. It contains no business
//! logic, no cognitive loops, and no network servers. Its sole responsibility
//! is to instantiate `astrid_events::EventBus`, load `.capsule` files into
//! the Extism sandbox, and route IPC bytes between them.

/// The Management API router listening to the `EventBus`.
pub mod kernel_router;
/// The Unix Domain Socket manager.
pub mod socket;

use astrid_audit::AuditLog;
use astrid_capabilities::{CapabilityStore, DirHandle};
use astrid_capsule::registry::CapsuleRegistry;
use astrid_core::SessionId;
use astrid_crypto::KeyPair;
use astrid_events::EventBus;
use astrid_mcp::{McpClient, SecureMcpClient, ServerManager, ServersConfig};
use astrid_vfs::{HostVfs, OverlayVfs, Vfs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::RwLock;

/// The core Operating System Kernel.
pub struct Kernel {
    /// The unique identifier for this kernel session.
    pub session_id: SessionId,
    /// The global IPC message bus.
    pub event_bus: Arc<EventBus>,
    /// The process manager (loaded WASM capsules).
    pub capsules: Arc<RwLock<CapsuleRegistry>>,
    /// The secure MCP client with capability-based authorization and audit logging.
    pub mcp: SecureMcpClient,
    /// The capability store for this session.
    pub capabilities: Arc<CapabilityStore>,
    /// The global Virtual File System mount.
    pub vfs: Arc<dyn Vfs>,
    /// Concrete reference to the [`OverlayVfs`] for commit/rollback operations.
    pub overlay_vfs: Arc<OverlayVfs>,
    /// Ephemeral upper directory for the overlay VFS. Kept alive for the
    /// kernel session lifetime; dropped on shutdown to discard uncommitted writes.
    _upper_dir: Arc<tempfile::TempDir>,
    /// The global physical root handle (cap-std) for the VFS.
    pub vfs_root_handle: DirHandle,
    /// The physical path the VFS is mounted to.
    pub workspace_root: PathBuf,
    /// The global shared resources directory (`~/.astrid/shared/`). Capsules
    /// declaring `fs_read = ["global://"]` can read files under this root.
    /// Scoped to `shared/` so that keys, databases, and capsule .env files in
    /// `~/.astrid/` are NOT accessible. Write access is intentionally not
    /// granted to any shipped capsule.
    ///
    /// Always `Some` in production (boot requires `AstridHome`). Remains
    /// `Option` for compatibility with `CapsuleContext` and test fixtures.
    pub global_root: Option<PathBuf>,
    /// The natively bound Unix Socket for the CLI proxy.
    pub cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    /// Shared KV store backing all capsule-scoped stores and kernel state.
    pub kv: Arc<astrid_storage::SurrealKvStore>,
    /// Chain-linked cryptographic audit log with persistent storage.
    pub audit_log: Arc<AuditLog>,
    /// Number of active client connections (CLI sessions).
    pub active_connections: AtomicUsize,
    /// Ephemeral mode: shut down immediately when the last client disconnects.
    pub ephemeral: AtomicBool,
    /// Instant when the kernel was booted (for uptime calculation).
    pub boot_time: std::time::Instant,
    /// Sender for the API-initiated shutdown signal. The daemon's main loop
    /// selects on the receiver to exit gracefully without `process::exit`.
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Session token for socket authentication. Generated at boot, written to
    /// `~/.astrid/run/system.token`. CLI sends this as its first message.
    pub session_token: Arc<astrid_core::session_token::SessionToken>,
    /// Path where the session token was written at boot. Stored so shutdown
    /// uses the exact same path (avoids fallback mismatch if env changes).
    token_path: PathBuf,
    /// Shared allowance store for capsule-level approval decisions.
    ///
    /// Capsules can check existing allowances and create new ones when
    /// users approve actions with session/always scope.
    pub allowance_store: Arc<astrid_approval::AllowanceStore>,
    /// System-wide identity store for platform user resolution.
    identity_store: Arc<dyn astrid_storage::IdentityStore>,
}

impl Kernel {
    /// Boot a new Kernel instance mounted at the specified directory.
    ///
    /// # Panics
    ///
    /// Panics if called on a single-threaded tokio runtime. The capsule
    /// system uses `block_in_place` which requires a multi-threaded runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the VFS mount paths cannot be registered.
    pub async fn new(
        session_id: SessionId,
        workspace_root: PathBuf,
    ) -> Result<Arc<Self>, std::io::Error> {
        use astrid_core::dirs::AstridHome;

        assert!(
            tokio::runtime::Handle::current().runtime_flavor()
                == tokio::runtime::RuntimeFlavor::MultiThread,
            "Kernel requires a multi-threaded tokio runtime (block_in_place panics on \
             single-threaded). Use #[tokio::main] or Runtime::new() instead of current_thread."
        );

        let event_bus = Arc::new(EventBus::new());
        let capsules = Arc::new(RwLock::new(CapsuleRegistry::new()));

        // Resolve the Astrid home directory. Required for persistent KV store
        // and audit log. Fails boot if neither $ASTRID_HOME nor $HOME is set.
        let home = AstridHome::resolve().map_err(|e| {
            std::io::Error::other(format!(
                "Failed to resolve Astrid home (set $ASTRID_HOME or $HOME): {e}"
            ))
        })?;

        // Resolve the global shared directory for the `global://` VFS scheme.
        // Points to `~/.astrid/home/{principal}/` — NOT the full `~/.astrid/`
        // root — so capsules cannot access keys, databases, or config.
        let default_principal = astrid_core::PrincipalId::default();
        let principal_home = home.principal_home(&default_principal);
        let global_root = Some(principal_home.root().to_path_buf());

        // 1. Open the persistent KV store (needed by capability store below).
        let kv_path = home.state_db_path();
        let kv = Arc::new(
            astrid_storage::SurrealKvStore::open(&kv_path)
                .map_err(|e| std::io::Error::other(format!("Failed to open KV store: {e}")))?,
        );
        // TODO: clear ephemeral keys (e: prefix) on boot when the key
        // lifecycle tier convention is established.

        // 2. Initialize MCP process manager with security layer.
        //    Set workspace_root so sandboxed MCP servers have a writable directory.
        let mcp_config = ServersConfig::load_default().unwrap_or_default();
        let mcp_manager =
            ServerManager::new(mcp_config).with_workspace_root(workspace_root.clone());
        let mcp_client = McpClient::new(mcp_manager);

        // 3. Bootstrap capability store (persistent) and audit log.
        //    Key rotation invalidates persisted tokens (fail-secure by design).
        let capabilities = Arc::new(
            CapabilityStore::with_kv_store(Arc::clone(&kv) as Arc<dyn astrid_storage::KvStore>)
                .map_err(|e| {
                    std::io::Error::other(format!("Failed to init capability store: {e}"))
                })?,
        );
        let audit_log = open_audit_log()?;
        let mcp = SecureMcpClient::new(
            mcp_client,
            Arc::clone(&capabilities),
            Arc::clone(&audit_log),
            session_id.clone(),
        );

        // 4. Establish the physical security boundary (sandbox handle)
        let root_handle = DirHandle::new();

        // 5. Initialize sandboxed overlay VFS (lower=workspace, upper=temp)
        let (overlay_vfs, upper_temp) = init_overlay_vfs(&root_handle, &workspace_root).await?;

        // 6. Bind the secure Unix socket and generate session token.
        // The socket is bound here, but not yet listened on. The token is
        // generated before any capsule can accept connections, preventing
        // a race where a client connects before the token file exists.
        let listener = socket::bind_session_socket()?;
        let (session_token, token_path) = socket::generate_session_token()?;

        let allowance_store = Arc::new(astrid_approval::AllowanceStore::new());

        // Create system-wide identity store backed by the shared KV.
        let identity_kv = astrid_storage::ScopedKvStore::new(
            Arc::clone(&kv) as Arc<dyn astrid_storage::KvStore>,
            "system:identity",
        )
        .map_err(|e| std::io::Error::other(format!("Failed to create identity KV: {e}")))?;
        let identity_store: Arc<dyn astrid_storage::IdentityStore> =
            Arc::new(astrid_storage::KvIdentityStore::new(identity_kv));

        // Bootstrap the CLI root user (idempotent).
        bootstrap_cli_root_user(&identity_store)
            .await
            .map_err(|e| {
                std::io::Error::other(format!("Failed to bootstrap CLI root user: {e}"))
            })?;

        // Apply pre-configured identity links from config.
        apply_identity_config(&identity_store, &workspace_root).await;

        let kernel = Arc::new(Self {
            session_id,
            event_bus,
            capsules,
            mcp,
            capabilities,
            vfs: Arc::clone(&overlay_vfs) as Arc<dyn Vfs>,
            overlay_vfs,
            _upper_dir: Arc::new(upper_temp),
            vfs_root_handle: root_handle,
            workspace_root,
            global_root,
            cli_socket_listener: Some(Arc::new(tokio::sync::Mutex::new(listener))),
            kv,
            audit_log,
            active_connections: AtomicUsize::new(0),
            ephemeral: AtomicBool::new(false),
            boot_time: std::time::Instant::now(),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            session_token: Arc::new(session_token),
            token_path,
            allowance_store,
            identity_store,
        });

        drop(kernel_router::spawn_kernel_router(Arc::clone(&kernel)));
        drop(spawn_idle_monitor(Arc::clone(&kernel)));
        drop(spawn_react_watchdog(Arc::clone(&kernel.event_bus)));
        drop(spawn_capsule_health_monitor(Arc::clone(&kernel)));

        // Spawn the event dispatcher — routes EventBus events to capsule interceptors
        let dispatcher = astrid_capsule::dispatcher::EventDispatcher::new(
            Arc::clone(&kernel.capsules),
            Arc::clone(&kernel.event_bus),
        );
        tokio::spawn(dispatcher.run());

        debug_assert_eq!(
            kernel.event_bus.subscriber_count(),
            INTERNAL_SUBSCRIBER_COUNT,
            "INTERNAL_SUBSCRIBER_COUNT is stale; update it when adding permanent subscribers"
        );

        Ok(kernel)
    }

    /// Load a capsule into the Kernel from a directory containing a Capsule.toml
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be loaded, the capsule cannot be created, or registration fails.
    async fn load_capsule(&self, dir: PathBuf) -> Result<(), anyhow::Error> {
        let manifest_path = dir.join("Capsule.toml");
        let manifest = astrid_capsule::discovery::load_manifest(&manifest_path)
            .map_err(|e| anyhow::anyhow!(e))?;

        let loader = astrid_capsule::loader::CapsuleLoader::new(self.mcp.clone());
        let mut capsule = loader.create_capsule(manifest, dir.clone())?;

        // Build the context — use the shared kernel KV so capsules can
        // communicate state through overlapping KV namespaces.
        let principal = astrid_core::PrincipalId::default();
        let kv = astrid_storage::ScopedKvStore::new(
            Arc::clone(&self.kv) as Arc<dyn astrid_storage::KvStore>,
            format!("{principal}:capsule:{}", capsule.id()),
        )?;

        // Pre-load `.env.json` into the KV store if it exists
        let env_path = dir.join(".env.json");
        if env_path.exists()
            && let Ok(contents) = std::fs::read_to_string(&env_path)
            && let Ok(env_map) =
                serde_json::from_str::<std::collections::HashMap<String, String>>(&contents)
        {
            for (k, v) in env_map {
                let _ = kv.set(&k, v.into_bytes()).await;
            }
        }

        let ctx = astrid_capsule::context::CapsuleContext::new(
            self.workspace_root.clone(),
            self.global_root.clone(),
            kv,
            Arc::clone(&self.event_bus),
            self.cli_socket_listener.clone(),
        )
        .with_registry(Arc::clone(&self.capsules))
        .with_session_token(Arc::clone(&self.session_token))
        .with_allowance_store(Arc::clone(&self.allowance_store))
        .with_identity_store(Arc::clone(&self.identity_store));

        capsule.load(&ctx).await?;

        let mut registry = self.capsules.write().await;
        registry
            .register(capsule)
            .map_err(|e| anyhow::anyhow!("Failed to register capsule: {e}"))?;

        Ok(())
    }

    /// Restart a capsule by unloading it and re-loading from its source directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the capsule has no source directory, cannot be
    /// unregistered, or fails to reload.
    async fn restart_capsule(
        &self,
        id: &astrid_capsule::capsule::CapsuleId,
    ) -> Result<(), anyhow::Error> {
        // Get source directory before unregistering.
        let source_dir = {
            let registry = self.capsules.read().await;
            let capsule = registry
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("capsule '{id}' not found in registry"))?;
            capsule
                .source_dir()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| anyhow::anyhow!("capsule '{id}' has no source directory"))?
        };

        // Unregister and explicitly unload. There is no Drop impl that
        // calls unload() (it's async), so we must do it here to avoid
        // leaking MCP subprocesses and other engine resources.
        let old_capsule = {
            let mut registry = self.capsules.write().await;
            registry
                .unregister(id)
                .map_err(|e| anyhow::anyhow!("failed to unregister capsule '{id}': {e}"))?
        };
        // Explicitly unload the old capsule. There is no Drop impl that
        // calls unload() (it's async), so we must do it here to avoid
        // leaking MCP subprocesses and other engine resources.
        // Arc::get_mut requires exclusive ownership (strong_count == 1).
        {
            let mut old = old_capsule;
            if let Some(capsule) = std::sync::Arc::get_mut(&mut old) {
                if let Err(e) = capsule.unload().await {
                    tracing::warn!(
                        capsule_id = %id,
                        error = %e,
                        "Capsule unload failed during restart"
                    );
                }
            } else {
                tracing::warn!(
                    capsule_id = %id,
                    "Cannot call unload during restart - Arc still held by in-flight task"
                );
            }
        }

        // Re-load from disk.
        self.load_capsule(source_dir).await?;

        // Signal the newly loaded capsule to clean up ephemeral state
        // from the previous incarnation. Capsules that don't implement
        // `handle_lifecycle_restart` will return an error, which is fine.
        //
        // Clone the capsule Arc under a brief read lock, then drop the
        // guard before invoke_interceptor which calls block_in_place.
        // Holding the RwLock across block_in_place parks the worker thread
        // and starves registry writers (health monitor, capsule loading).
        let capsule = {
            let registry = self.capsules.read().await;
            registry.get(id)
        };
        if let Some(capsule) = capsule
            && let Err(e) = capsule.invoke_interceptor("handle_lifecycle_restart", &[])
        {
            tracing::debug!(
                capsule_id = %id,
                error = %e,
                "Capsule does not handle lifecycle restart (optional)"
            );
        }

        Ok(())
    }

    /// Auto-discover and load all capsules from the standard directories (`~/.astrid/capsules` and `.astrid/capsules`).
    ///
    /// Capsules are loaded in dependency order (topological sort) with
    /// uplink/daemon capsules loaded first. Each uplink must signal
    /// readiness before non-uplink capsules are loaded.
    ///
    /// After all capsules are loaded, tool schemas are injected into every
    /// capsule's KV namespace and the `astrid.v1.capsules_loaded` event is published.
    pub async fn load_all_capsules(&self) {
        use astrid_capsule::toposort::toposort_manifests;
        use astrid_core::dirs::AstridHome;

        // Discovery paths in priority order: system > principal > workspace.
        let mut paths = Vec::new();
        if let Ok(home) = AstridHome::resolve() {
            paths.push(home.capsules_dir()); // system capsules (highest priority)
            let principal = astrid_core::PrincipalId::default();
            paths.push(home.principal_home(&principal).capsules_dir()); // principal capsules
        }

        let discovered = astrid_capsule::discovery::discover_manifests(Some(&paths));

        // Topological sort ALL capsules together so cross-partition
        // requirements (e.g. a non-uplink requiring an uplink's capability)
        // resolve correctly without spurious "not provided" warnings.
        let sorted = match toposort_manifests(discovered) {
            Ok(sorted) => sorted,
            Err((e, original)) => {
                tracing::error!(
                    cycle = %e,
                    "Dependency cycle in capsules, falling back to discovery order"
                );
                original
            },
        };

        // Defence-in-depth: manifest validation in discovery.rs rejects
        // uplinks with `requires`, but warn here in case a manifest bypasses
        // the normal load path.
        for (manifest, _) in &sorted {
            if manifest.capabilities.uplink && !manifest.dependencies.requires.is_empty() {
                tracing::warn!(
                    capsule = %manifest.package.name,
                    requires = ?manifest.dependencies.requires,
                    "Uplink capsule has [dependencies].requires - \
                     this should have been rejected at manifest load time"
                );
            }
        }

        // Partition after sorting: uplinks first, then the rest.
        // The relative order within each partition is preserved from the
        // toposort, so dependency edges are still respected. Cross-partition
        // edges (non-uplink requiring an uplink) are satisfied by construction
        // since all uplinks load first. The inverse (uplink requiring a
        // non-uplink) is rejected above.
        let (uplinks, others): (Vec<_>, Vec<_>) =
            sorted.into_iter().partition(|(m, _)| m.capabilities.uplink);

        // Load uplinks first so their event bus subscriptions are ready.
        let uplink_names: Vec<String> = uplinks
            .iter()
            .map(|(m, _)| m.package.name.clone())
            .collect();
        for (manifest, dir) in &uplinks {
            if let Err(e) = self.load_capsule(dir.clone()).await {
                tracing::warn!(
                    capsule = %manifest.package.name,
                    error = %e,
                    "Failed to load uplink capsule during discovery"
                );
            }
        }

        // Wait for uplink capsules to signal readiness before loading
        // non-uplink capsules. This ensures IPC subscriptions are active.
        self.await_capsule_readiness(&uplink_names).await;

        for (manifest, dir) in &others {
            if let Err(e) = self.load_capsule(dir.clone()).await {
                tracing::warn!(
                    capsule = %manifest.package.name,
                    error = %e,
                    "Failed to load capsule during discovery"
                );
            }
        }

        // Wait for non-uplink run-loop capsules too, so any future
        // dependency edges between them are respected.
        let other_names: Vec<String> = others.iter().map(|(m, _)| m.package.name.clone()).collect();
        self.await_capsule_readiness(&other_names).await;

        // Inject tool schemas into every capsule's KV namespace so any
        // capsule (e.g. react) can read them via kv::get_json("tool_schemas").
        self.inject_tool_schemas().await;

        // Signal that all capsules have been loaded so uplink capsules
        // (like the registry) can proceed with discovery instead of
        // polling with arbitrary timeouts.
        let msg = astrid_events::ipc::IpcMessage::new(
            "astrid.v1.capsules_loaded",
            astrid_events::ipc::IpcPayload::RawJson(serde_json::json!({"status": "ready"})),
            self.session_id.0,
        );
        let _ = self.event_bus.publish(astrid_events::AstridEvent::Ipc {
            metadata: astrid_events::EventMetadata::new("kernel"),
            message: msg,
        });
    }

    /// Record that a new client connection has been established.
    pub fn connection_opened(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a client connection has been closed.
    ///
    /// Uses `fetch_update` for atomic saturating decrement - avoids the TOCTOU
    /// window where `fetch_sub` wraps to `usize::MAX` before a corrective store.
    ///
    /// When the last connection closes (counter reaches 0), clears all
    /// session-scoped allowances so they don't leak into the next CLI session.
    pub fn connection_closed(&self) {
        let result =
            self.active_connections
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    if n == 0 {
                        None
                    } else {
                        Some(n.saturating_sub(1))
                    }
                });

        // Previous value was 1 -> now 0: last client disconnected.
        // Clear session-scoped allowances so they don't leak into the next session.
        if result == Ok(1) {
            self.allowance_store.clear_session_allowances();
            tracing::info!("last client disconnected, session allowances cleared");
        }
    }

    /// Enable or disable ephemeral mode (immediate shutdown on last disconnect).
    pub fn set_ephemeral(&self, val: bool) {
        self.ephemeral.store(val, Ordering::Relaxed);
    }

    /// Number of active client connections.
    pub fn connection_count(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    /// Gracefully shut down the kernel.
    ///
    /// 1. Publish `KernelShutdown` event on the bus.
    /// 2. Drain and unload all capsules (stops MCP child processes, WASM engines).
    /// 3. Flush and close the persistent KV store.
    /// 4. Remove the Unix socket file.
    pub async fn shutdown(&self, reason: Option<String>) {
        tracing::info!(reason = ?reason, "Kernel shutting down");

        // 1. Notify all subscribers so capsules can react.
        let _ = self
            .event_bus
            .publish(astrid_events::AstridEvent::KernelShutdown {
                metadata: astrid_events::EventMetadata::new("kernel"),
                reason: reason.clone(),
            });

        // 2. Drain the registry so the dispatcher cannot hand out new Arc clones,
        // then unload each capsule. MCP engine unload is critical - it calls
        // `mcp_client.disconnect()` to gracefully terminate child processes.
        // Without explicit unload, MCP child processes become orphaned.
        //
        // The `EventDispatcher` temporarily clones `Arc<dyn Capsule>` into
        // spawned interceptor tasks. After draining, no new clones can be
        // created, but in-flight tasks may still hold references. We retry
        // `Arc::get_mut` with brief yields to let them complete.
        let capsules = {
            let mut reg = self.capsules.write().await;
            reg.drain()
        };
        for mut arc in capsules {
            let id = arc.id().clone();
            let mut unloaded = false;

            for retry in 0..20_u32 {
                if let Some(capsule) = Arc::get_mut(&mut arc) {
                    if let Err(e) = capsule.unload().await {
                        tracing::warn!(
                            capsule_id = %id,
                            error = %e,
                            "Failed to unload capsule during shutdown"
                        );
                    }
                    unloaded = true;
                    break;
                }
                if retry < 19 {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }

            if !unloaded {
                tracing::warn!(
                    capsule_id = %id,
                    strong_count = Arc::strong_count(&arc),
                    "Dropping capsule without explicit unload after retries exhausted; \
                     MCP child processes may be orphaned"
                );
            }
            drop(arc);
        }

        // 3. Flush the persistent KV store.
        if let Err(e) = self.kv.close().await {
            tracing::warn!(error = %e, "Failed to flush KV store during shutdown");
        }

        // 4. Remove the socket and token files so stale-socket detection works
        // on next boot and the auth token doesn't persist on disk after shutdown.
        // This runs AFTER capsule unload, which is the correct order: MCP child
        // processes communicate via stdio pipes (not this Unix socket), so they
        // are already terminated by step 2. The socket is only used for
        // CLI-to-kernel IPC.
        let socket_path = crate::socket::kernel_socket_path();
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&self.token_path);
        crate::socket::remove_readiness_file();

        tracing::info!("Kernel shutdown complete");
    }

    /// Wait for a set of capsules to signal readiness, in parallel.
    ///
    /// Collects `Arc<dyn Capsule>` handles under a short-lived read lock,
    /// then drops the lock before awaiting. Capsules without a run loop
    /// return `Ready` immediately and don't contribute to wait time.
    async fn await_capsule_readiness(&self, names: &[String]) {
        use astrid_capsule::capsule::ReadyStatus;

        if names.is_empty() {
            return;
        }

        let timeout = std::time::Duration::from_millis(500);
        let capsules: Vec<(String, std::sync::Arc<dyn astrid_capsule::capsule::Capsule>)> = {
            let registry = self.capsules.read().await;
            names
                .iter()
                .filter_map(
                    |name| match astrid_capsule::capsule::CapsuleId::new(name.clone()) {
                        Ok(capsule_id) => registry.get(&capsule_id).map(|c| (name.clone(), c)),
                        Err(e) => {
                            tracing::warn!(
                                capsule = %name,
                                error = %e,
                                "Invalid capsule ID, skipping readiness wait"
                            );
                            None
                        },
                    },
                )
                .collect()
        };

        // Await all capsules concurrently - independent capsules shouldn't
        // compound each other's timeout.
        let mut set = tokio::task::JoinSet::new();
        for (name, capsule) in capsules {
            set.spawn(async move {
                let status = capsule.wait_ready(timeout).await;
                (name, status)
            });
        }
        while let Some(result) = set.join_next().await {
            if let Ok((name, status)) = result {
                match status {
                    ReadyStatus::Ready => {},
                    ReadyStatus::Timeout => {
                        tracing::warn!(
                            capsule = %name,
                            timeout_ms = timeout.as_millis(),
                            "Capsule did not signal ready within timeout"
                        );
                    },
                    ReadyStatus::Crashed => {
                        tracing::error!(
                            capsule = %name,
                            "Capsule run loop exited before signaling ready"
                        );
                    },
                }
            }
        }
    }

    /// Collect all tool definitions from loaded capsule manifests and write
    /// them to every capsule's scoped KV namespace as `tool_schemas`.
    async fn inject_tool_schemas(&self) {
        use astrid_events::llm::LlmToolDefinition;
        use astrid_storage::KvStore;

        // Collect tools and capsule IDs under a short-lived read lock,
        // then drop the guard before the async KV write loop. Holding
        // the RwLock across N awaits would block all registry writers.
        let (all_tools, capsule_ids) = {
            let registry = self.capsules.read().await;
            // Collect tools declared in Capsule.toml manifests (WASM capsules).
            let mut tools: Vec<LlmToolDefinition> = registry
                .values()
                .flat_map(|capsule| {
                    capsule.manifest().tools.iter().map(|t| LlmToolDefinition {
                        name: t.name.clone(),
                        description: Some(t.description.clone()),
                        input_schema: t.input_schema.clone(),
                    })
                })
                .collect();
            let ids: Vec<String> = registry.list().iter().map(ToString::to_string).collect();

            // Also collect tools discovered from MCP host engines (Tier 2
            // OpenClaw capsules, legacy MCP servers). These are registered
            // dynamically at runtime via the MCP bridge.
            let mut tool_names: std::collections::HashSet<String> =
                tools.iter().map(|t| t.name.clone()).collect();
            let mcp_tools = self.mcp.inner().server_manager().all_tools().await;
            for t in mcp_tools {
                // Avoid duplicates if a tool is declared in both manifest and MCP.
                if tool_names.insert(t.name.clone()) {
                    // Ensure input_schema has "properties" — LLM APIs require it.
                    let mut schema = t.input_schema;
                    if let Some(obj) = schema.as_object_mut() {
                        obj.entry("properties")
                            .or_insert_with(|| serde_json::json!({}));
                    }
                    tools.push(LlmToolDefinition {
                        name: t.name,
                        description: t.description,
                        input_schema: schema,
                    });
                }
            }

            (tools, ids)
        };

        if all_tools.is_empty() {
            return;
        }

        let tool_bytes = match serde_json::to_vec(&all_tools) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize tool schemas");
                return;
            },
        };

        tracing::info!(
            tool_count = all_tools.len(),
            "Injecting tool schemas into capsule KV stores"
        );

        let principal = astrid_core::PrincipalId::default();
        for capsule_id in &capsule_ids {
            let namespace = format!("{principal}:capsule:{capsule_id}");
            if let Err(e) = self
                .kv
                .set(&namespace, "tool_schemas", tool_bytes.clone())
                .await
            {
                tracing::warn!(
                    capsule = %capsule_id,
                    error = %e,
                    "Failed to inject tool schemas"
                );
            }
        }
    }
}

/// Open (or create) the persistent audit log and verify historical chain integrity.
///
/// Initialize the sandboxed overlay VFS.
///
/// Creates a lower (read-only workspace) and upper (session-scoped temp dir)
/// layer, returning the overlay and the `TempDir` whose lifetime keeps the
/// upper layer alive.
async fn init_overlay_vfs(
    root_handle: &DirHandle,
    workspace_root: &Path,
) -> Result<(Arc<OverlayVfs>, tempfile::TempDir), std::io::Error> {
    let lower_vfs = HostVfs::new();
    lower_vfs
        .register_dir(root_handle.clone(), workspace_root.to_path_buf())
        .await
        .map_err(|_| std::io::Error::other("Failed to register lower vfs dir"))?;

    let upper_temp = tempfile::TempDir::new()
        .map_err(|e| std::io::Error::other(format!("Failed to create overlay temp dir: {e}")))?;
    let upper_vfs = HostVfs::new();
    upper_vfs
        .register_dir(root_handle.clone(), upper_temp.path().to_path_buf())
        .await
        .map_err(|_| std::io::Error::other("Failed to register upper vfs dir"))?;

    let overlay = Arc::new(OverlayVfs::new(Box::new(lower_vfs), Box::new(upper_vfs)));
    Ok((overlay, upper_temp))
}

/// Loads the runtime signing key from `~/.astrid/keys/runtime.key`, generating a
/// new one if it doesn't exist. Opens the `SurrealKV`-backed audit database at
/// `~/.astrid/audit.db` and runs `verify_all()` to detect any tampering of
/// historical entries. Verification failures are logged at `error!` level but
/// do not block boot (fail-open for availability, loud alert for integrity).
fn open_audit_log() -> std::io::Result<Arc<AuditLog>> {
    use astrid_core::dirs::AstridHome;

    let home = AstridHome::resolve()
        .map_err(|e| std::io::Error::other(format!("cannot resolve Astrid home: {e}")))?;
    home.ensure()
        .map_err(|e| std::io::Error::other(format!("cannot create Astrid home dirs: {e}")))?;

    let runtime_key = load_or_generate_runtime_key(&home.keys_dir())?;
    let default_principal = astrid_core::PrincipalId::default();
    let principal_home = home.principal_home(&default_principal);
    principal_home
        .ensure()
        .map_err(|e| std::io::Error::other(format!("cannot create principal home dirs: {e}")))?;
    let audit_log = AuditLog::open(principal_home.audit_dir(), runtime_key)
        .map_err(|e| std::io::Error::other(format!("cannot open audit log: {e}")))?;

    // Verify all historical chains on boot.
    match audit_log.verify_all() {
        Ok(results) => {
            let total_sessions = results.len();
            let mut tampered_sessions: usize = 0;

            for (session_id, result) in &results {
                if !result.valid {
                    tampered_sessions = tampered_sessions.saturating_add(1);
                    for issue in &result.issues {
                        tracing::error!(
                            session_id = %session_id,
                            issue = %issue,
                            "Audit chain integrity violation detected"
                        );
                    }
                }
            }

            if tampered_sessions > 0 {
                tracing::error!(
                    total_sessions,
                    tampered_sessions,
                    "Audit chain verification found tampered sessions"
                );
            } else if total_sessions > 0 {
                tracing::info!(
                    total_sessions,
                    "Audit chain verification passed for all sessions"
                );
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "Audit chain verification failed to run");
        },
    }

    Ok(Arc::new(audit_log))
}

/// Load the runtime ed25519 signing key from disk, or generate and persist a new one.
///
/// The key file is 32 bytes of raw secret key material at `{keys_dir}/runtime.key`.
fn load_or_generate_runtime_key(keys_dir: &Path) -> std::io::Result<KeyPair> {
    let key_path = keys_dir.join("runtime.key");

    if key_path.exists() {
        let bytes = std::fs::read(&key_path)?;
        KeyPair::from_secret_key(&bytes).map_err(|e| {
            std::io::Error::other(format!(
                "invalid runtime key at {}: {e}",
                key_path.display()
            ))
        })
    } else {
        let keypair = KeyPair::generate();
        std::fs::create_dir_all(keys_dir)?;
        std::fs::write(&key_path, keypair.secret_key_bytes())?;

        // Secure permissions (owner-only) on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        tracing::info!(key_id = %keypair.key_id_hex(), "Generated new runtime signing key");
        Ok(keypair)
    }
}

/// Spawns a background task that cleanly shuts down the Kernel if there is no activity.
///
/// Uses dual-signal idle detection:
/// - **Primary:** explicit `active_connections` counter (incremented on first IPC
///   message per source, decremented on `Disconnect`).
/// - **Secondary:** `EventBus::subscriber_count()` minus the kernel router's own
///   subscription. When a CLI process dies without sending `Disconnect`, its
///   broadcast receiver is dropped so the subscriber count falls.
///
/// Takes the minimum of both signals to handle ungraceful disconnects.
///
/// Configurable via `ASTRID_IDLE_TIMEOUT_SECS` (default 300 = 5 minutes).
/// Number of permanent internal event bus subscribers that are not client
/// connections: `KernelRouter` (`kernel.request.*`), `ConnectionTracker` (`client.*`),
/// and `EventDispatcher` (all events).
const INTERNAL_SUBSCRIBER_COUNT: usize = 3;

/// Initial grace period before idle checking begins.
const IDLE_INITIAL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
/// Additional grace for non-ephemeral daemons to let capsules fully initialize.
const IDLE_NON_EPHEMERAL_GRACE: std::time::Duration = std::time::Duration::from_secs(25);
/// How often the idle monitor polls when running in ephemeral mode.
const IDLE_EPHEMERAL_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
/// How often the idle monitor polls when running in persistent mode.
const IDLE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
/// Default idle timeout for non-ephemeral daemons (5 minutes).
const IDLE_DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);

fn spawn_idle_monitor(kernel: Arc<Kernel>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Initial grace period — wait for capsules to boot and first client
        // to connect before checking idle status.
        tokio::time::sleep(IDLE_INITIAL_GRACE).await;

        // Read ephemeral flag after grace period (set by daemon after boot).
        let ephemeral = kernel.ephemeral.load(Ordering::Relaxed);
        let idle_timeout = if ephemeral {
            std::time::Duration::ZERO
        } else {
            std::env::var("ASTRID_IDLE_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map_or(IDLE_DEFAULT_TIMEOUT, std::time::Duration::from_secs)
        };
        let check_interval = if ephemeral {
            IDLE_EPHEMERAL_CHECK_INTERVAL
        } else {
            IDLE_CHECK_INTERVAL
        };

        // Non-ephemeral: additional grace to let capsules fully initialize.
        if !ephemeral {
            tokio::time::sleep(IDLE_NON_EPHEMERAL_GRACE).await;
        }
        let mut idle_since: Option<tokio::time::Instant> = None;

        loop {
            tokio::time::sleep(check_interval).await;

            let connections = kernel.connection_count();

            // Secondary signal: broadcast subscriber count. Subtract the
            // permanent internal subscribers: KernelRouter (kernel.request.*),
            // ConnectionTracker (client.*), and EventDispatcher (all events).
            let bus_subscribers = kernel
                .event_bus
                .subscriber_count()
                .saturating_sub(INTERNAL_SUBSCRIBER_COUNT);

            // Take the minimum: if a CLI died without Disconnect, the counter
            // stays inflated but the subscriber count drops.
            let effective_connections = connections.min(bus_subscribers);

            let has_daemons = {
                let reg = kernel.capsules.read().await;
                reg.values().any(|c| {
                    let m = c.manifest();
                    !m.uplinks.is_empty() || !m.cron_jobs.is_empty()
                })
            };

            if effective_connections == 0 && !has_daemons {
                let now = tokio::time::Instant::now();
                let start = *idle_since.get_or_insert(now);
                let elapsed = now.duration_since(start);

                tracing::debug!(
                    idle_secs = elapsed.as_secs(),
                    timeout_secs = idle_timeout.as_secs(),
                    connections,
                    bus_subscribers,
                    "Kernel idle, monitoring timeout"
                );

                if elapsed >= idle_timeout {
                    tracing::info!("Idle timeout reached, initiating shutdown");
                    kernel.shutdown(Some("idle_timeout".to_string())).await;
                    std::process::exit(0);
                }
            } else {
                if idle_since.is_some() {
                    tracing::debug!(
                        effective_connections,
                        has_daemons,
                        "Activity detected, resetting idle timer"
                    );
                }
                idle_since = None;
            }
        }
    })
}

/// Tracks restart attempts for a single capsule with exponential backoff.
struct RestartTracker {
    attempts: u32,
    last_attempt: std::time::Instant,
    backoff: std::time::Duration,
}

impl RestartTracker {
    const MAX_ATTEMPTS: u32 = 5;
    const INITIAL_BACKOFF: std::time::Duration = std::time::Duration::from_secs(2);
    const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(120);

    fn new() -> Self {
        Self {
            attempts: 0,
            last_attempt: std::time::Instant::now(),
            backoff: Self::INITIAL_BACKOFF,
        }
    }

    /// Returns `true` if a restart should be attempted now.
    fn should_restart(&self) -> bool {
        self.attempts < Self::MAX_ATTEMPTS && self.last_attempt.elapsed() >= self.backoff
    }

    /// Record a restart attempt and advance the backoff.
    fn record_attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
        self.last_attempt = std::time::Instant::now();
        self.backoff = self.backoff.saturating_mul(2).min(Self::MAX_BACKOFF);
    }

    /// Returns `true` if all retry attempts have been exhausted.
    fn exhausted(&self) -> bool {
        self.attempts >= Self::MAX_ATTEMPTS
    }
}

/// Attempts to restart a failed capsule, respecting backoff and max retries.
///
/// Returns `true` if the tracker should be removed (successful restart).
async fn attempt_capsule_restart(
    kernel: &Kernel,
    id_str: &str,
    tracker: &mut RestartTracker,
) -> bool {
    if tracker.exhausted() {
        return false;
    }

    if !tracker.should_restart() {
        tracing::debug!(
            capsule_id = %id_str,
            next_attempt_in = ?tracker.backoff.saturating_sub(tracker.last_attempt.elapsed()),
            "Waiting for backoff before next restart attempt"
        );
        return false;
    }

    tracker.record_attempt();
    let attempt = tracker.attempts;

    tracing::warn!(
        capsule_id = %id_str,
        attempt,
        max_attempts = RestartTracker::MAX_ATTEMPTS,
        "Attempting capsule restart"
    );

    let capsule_id = astrid_capsule::capsule::CapsuleId::from_static(id_str);
    match kernel.restart_capsule(&capsule_id).await {
        Ok(()) => {
            tracing::info!(capsule_id = %id_str, attempt, "Capsule restarted successfully");
            true
        },
        Err(e) => {
            tracing::error!(capsule_id = %id_str, attempt, error = %e, "Capsule restart failed");
            if tracker.exhausted() {
                tracing::error!(
                    capsule_id = %id_str,
                    "All restart attempts exhausted - capsule will remain down"
                );
            }
            false
        },
    }
}

/// Spawns a background task that periodically probes capsule health.
///
/// Every 10 seconds, reads the capsule registry and calls `check_health()` on
/// each capsule that is currently in `Ready` state. If a capsule reports
/// `Failed`, attempts to restart it with exponential backoff (max 5 attempts).
/// Publishes `astrid.v1.health.failed` IPC events for each detected failure.
fn spawn_capsule_health_monitor(kernel: Arc<Kernel>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        interval.tick().await; // Skip the first immediate tick.

        let mut restart_trackers: std::collections::HashMap<String, RestartTracker> =
            std::collections::HashMap::new();

        loop {
            interval.tick().await;

            // Collect ready capsules under a brief read lock, then drop
            // the lock before calling check_health() or publishing events.
            let ready_capsules: Vec<std::sync::Arc<dyn astrid_capsule::capsule::Capsule>> = {
                let registry = kernel.capsules.read().await;
                registry
                    .list()
                    .into_iter()
                    .filter_map(|id| {
                        let capsule = registry.get(id)?;
                        if capsule.state() == astrid_capsule::capsule::CapsuleState::Ready {
                            Some(capsule)
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            // Probe health once per capsule, collect failures, then drop
            // the Arc Vec before restarting. This ensures restart_capsule's
            // Arc::get_mut can succeed (no other strong references held).
            let mut failures: Vec<(String, String)> = Vec::new();
            for capsule in &ready_capsules {
                let health = capsule.check_health();
                if let astrid_capsule::capsule::CapsuleState::Failed(reason) = health {
                    let id_str = capsule.id().to_string();
                    tracing::error!(capsule_id = %id_str, reason = %reason, "Capsule health check failed");

                    let msg = astrid_events::ipc::IpcMessage::new(
                        "astrid.v1.health.failed",
                        astrid_events::ipc::IpcPayload::Custom {
                            data: serde_json::json!({
                                "capsule_id": &id_str,
                                "reason": &reason,
                            }),
                        },
                        uuid::Uuid::new_v4(),
                    );
                    let _ = kernel.event_bus.publish(astrid_events::AstridEvent::Ipc {
                        metadata: astrid_events::EventMetadata::new("kernel"),
                        message: msg,
                    });
                    failures.push((id_str, reason));
                }
            }

            // Drop all Arc clones so restart_capsule's Arc::get_mut can
            // obtain exclusive access for calling unload().
            drop(ready_capsules);

            let failed_this_tick: std::collections::HashSet<&str> =
                failures.iter().map(|(id, _)| id.as_str()).collect();

            let mut restarted = Vec::new();
            for (id_str, _reason) in &failures {
                let tracker = restart_trackers
                    .entry(id_str.clone())
                    .or_insert_with(RestartTracker::new);

                if attempt_capsule_restart(&kernel, id_str, tracker).await {
                    restarted.push(id_str.clone());
                }
            }

            // Remove trackers for successfully restarted capsules.
            for id in &restarted {
                restart_trackers.remove(id);
            }

            // Prune trackers for capsules that recovered (healthy this tick).
            // Keep exhausted trackers and trackers still in their backoff
            // window (capsule may have been unregistered by a failed restart
            // attempt and won't appear in ready_capsules next tick).
            restart_trackers.retain(|id, tracker| {
                if tracker.exhausted() {
                    return true;
                }
                // Keep if still within backoff - the capsule may be absent
                // from the registry after a failed reload.
                if tracker.last_attempt.elapsed() < tracker.backoff {
                    return true;
                }
                failed_this_tick.contains(id.as_str())
            });
        }
    })
}

/// Spawns a periodic watchdog that publishes `astrid.v1.watchdog.tick` events every 5 seconds.
///
/// The `ReAct` capsule (WASM guest) cannot use async timers, so this kernel-side task
/// drives timeout enforcement by waking the capsule on a fixed interval. Each tick
/// causes the capsule's `handle_watchdog_tick` interceptor to run `check_phase_timeout`.
fn spawn_react_watchdog(event_bus: Arc<EventBus>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        // The first tick fires immediately - skip it to give capsules time to load.
        interval.tick().await;

        loop {
            interval.tick().await;

            let msg = astrid_events::ipc::IpcMessage::new(
                "astrid.v1.watchdog.tick",
                astrid_events::ipc::IpcPayload::Custom {
                    data: serde_json::json!({}),
                },
                uuid::Uuid::new_v4(),
            );
            let _ = event_bus.publish(astrid_events::AstridEvent::Ipc {
                metadata: astrid_events::EventMetadata::new("kernel"),
                message: msg,
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_or_generate_creates_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");

        let keypair = load_or_generate_runtime_key(&keys_dir).unwrap();
        let key_path = keys_dir.join("runtime.key");

        // Key file should exist with 32 bytes.
        assert!(key_path.exists());
        let bytes = std::fs::read(&key_path).unwrap();
        assert_eq!(bytes.len(), 32);

        // The written bytes should reconstruct the same public key.
        let reloaded = KeyPair::from_secret_key(&bytes).unwrap();
        assert_eq!(
            keypair.public_key_bytes(),
            reloaded.public_key_bytes(),
            "reloaded key should match generated key"
        );
    }

    #[test]
    fn test_load_or_generate_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");

        let first = load_or_generate_runtime_key(&keys_dir).unwrap();
        let second = load_or_generate_runtime_key(&keys_dir).unwrap();

        assert_eq!(
            first.public_key_bytes(),
            second.public_key_bytes(),
            "loading the same key file should produce the same keypair"
        );
    }

    #[test]
    fn test_load_or_generate_rejects_bad_key_length() {
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        std::fs::create_dir_all(&keys_dir).unwrap();

        // Write a key file with wrong length.
        std::fs::write(keys_dir.join("runtime.key"), [0u8; 16]).unwrap();

        let result = load_or_generate_runtime_key(&keys_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid runtime key"),
            "expected 'invalid runtime key' error, got: {err}"
        );
    }

    #[test]
    fn test_connection_counter_increment_decrement() {
        let counter = AtomicUsize::new(0);

        // Simulate connection_opened (fetch_add)
        counter.fetch_add(1, Ordering::Relaxed);
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 2);

        // Simulate connection_closed using the same fetch_update logic
        // as the real implementation to exercise the actual code path.
        for expected in [1, 0] {
            let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                if n == 0 {
                    None
                } else {
                    Some(n.saturating_sub(1))
                }
            });
            assert_eq!(counter.load(Ordering::Relaxed), expected);
        }
    }

    #[test]
    fn test_connection_counter_underflow_guard() {
        // Test the saturating behavior: decrementing from 0 should stay at 0.
        // Mirrors the fetch_update logic in connection_closed().
        let counter = AtomicUsize::new(0);

        let result = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            if n == 0 { None } else { Some(n - 1) }
        });
        // fetch_update returns Err(0) when the closure returns None (no-op).
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    /// Mirrors the `connection_closed()` logic: only `Ok(1)` (previous value 1,
    /// now 0) triggers `clear_session_allowances`. Update this test if
    /// `connection_closed()` is refactored.
    #[test]
    fn test_last_disconnect_clears_session_allowances() {
        use astrid_approval::AllowanceStore;
        use astrid_approval::allowance::{Allowance, AllowanceId, AllowancePattern};
        use astrid_core::types::Timestamp;
        use astrid_crypto::KeyPair;

        let store = AllowanceStore::new();
        let keypair = KeyPair::generate();

        // Session-only allowance (should be cleared on last disconnect).
        store
            .add_allowance(Allowance {
                id: AllowanceId::new(),
                action_pattern: AllowancePattern::ServerTools {
                    server: "session-server".to_string(),
                },
                created_at: Timestamp::now(),
                expires_at: None,
                max_uses: None,
                uses_remaining: None,
                session_only: true,
                workspace_root: None,
                signature: keypair.sign(b"test"),
            })
            .unwrap();

        // Persistent allowance (should survive).
        store
            .add_allowance(Allowance {
                id: AllowanceId::new(),
                action_pattern: AllowancePattern::ServerTools {
                    server: "persistent-server".to_string(),
                },
                created_at: Timestamp::now(),
                expires_at: None,
                max_uses: None,
                uses_remaining: None,
                session_only: false,
                workspace_root: None,
                signature: keypair.sign(b"test"),
            })
            .unwrap();

        assert_eq!(store.count(), 2);

        let counter = AtomicUsize::new(2);
        let simulate_disconnect = || {
            let result = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                if n == 0 {
                    None
                } else {
                    Some(n.saturating_sub(1))
                }
            });
            if result == Ok(1) {
                store.clear_session_allowances();
            }
        };

        // Two connections active. First disconnect: 2 -> 1 (not last).
        simulate_disconnect();
        assert_eq!(
            store.count(),
            2,
            "both allowances should survive non-final disconnect"
        );

        // Second disconnect: 1 -> 0 (last client gone).
        simulate_disconnect();
        assert_eq!(
            store.count(),
            1,
            "session allowance should be cleared on last disconnect"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_load_or_generate_sets_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");

        let _ = load_or_generate_runtime_key(&keys_dir).unwrap();

        let key_path = keys_dir.join("runtime.key");
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "key file should have 0o600 permissions, got {mode:#o}"
        );
    }

    #[test]
    fn restart_tracker_initial_state() {
        let tracker = RestartTracker::new();
        assert!(!tracker.exhausted());
        // Should not restart immediately (backoff hasn't elapsed).
        assert!(!tracker.should_restart());
    }

    #[test]
    fn restart_tracker_allows_restart_after_backoff() {
        let mut tracker = RestartTracker::new();
        // Simulate time passing by setting last_attempt in the past.
        tracker.last_attempt = std::time::Instant::now()
            - RestartTracker::INITIAL_BACKOFF
            - std::time::Duration::from_millis(1);
        assert!(tracker.should_restart());
    }

    #[test]
    fn restart_tracker_doubles_backoff() {
        let mut tracker = RestartTracker::new();
        assert_eq!(tracker.backoff, RestartTracker::INITIAL_BACKOFF);

        tracker.record_attempt();
        assert_eq!(
            tracker.backoff,
            RestartTracker::INITIAL_BACKOFF.saturating_mul(2)
        );
        assert_eq!(tracker.attempts, 1);

        tracker.record_attempt();
        assert_eq!(
            tracker.backoff,
            RestartTracker::INITIAL_BACKOFF.saturating_mul(4)
        );
        assert_eq!(tracker.attempts, 2);
    }

    #[test]
    fn restart_tracker_backoff_caps_at_max() {
        let mut tracker = RestartTracker::new();
        for _ in 0..20 {
            tracker.record_attempt();
        }
        assert_eq!(tracker.backoff, RestartTracker::MAX_BACKOFF);
    }

    #[test]
    fn restart_tracker_exhausted_at_max_attempts() {
        let mut tracker = RestartTracker::new();
        for _ in 0..RestartTracker::MAX_ATTEMPTS {
            assert!(!tracker.exhausted());
            tracker.record_attempt();
        }
        assert!(tracker.exhausted());
    }

    #[test]
    fn restart_tracker_should_restart_false_when_exhausted() {
        let mut tracker = RestartTracker::new();
        for _ in 0..RestartTracker::MAX_ATTEMPTS {
            tracker.record_attempt();
        }
        // Even if backoff has elapsed, exhausted tracker should not restart.
        tracker.last_attempt = std::time::Instant::now() - RestartTracker::MAX_BACKOFF;
        assert!(!tracker.should_restart());
    }
}

// ---------------------------------------------------------------------------
// Identity bootstrap helpers
// ---------------------------------------------------------------------------

/// Bootstrap the CLI root user identity at kernel boot.
///
/// Creates a deterministic root `AstridUserId` on first boot, or reloads it
/// on subsequent boots. Auto-links with `platform="cli"`,
/// `platform_user_id="local"`, `method="system"`.
///
/// Idempotent: skips creation if the root user already exists.
async fn bootstrap_cli_root_user(
    store: &Arc<dyn astrid_storage::IdentityStore>,
) -> Result<(), astrid_storage::IdentityError> {
    // Check if root user already exists by trying to resolve the CLI link.
    if let Some(_user) = store.resolve("cli", "local").await? {
        tracing::debug!("CLI root user already linked");
        return Ok(());
    }

    // No CLI link exists. Create or find the root user.
    let user = store.create_user(Some("root")).await?;
    tracing::info!(user_id = %user.id, "Created CLI root user");

    // Link the CLI platform identity.
    store.link("cli", "local", user.id, "system").await?;
    tracing::info!(user_id = %user.id, "Linked CLI root user (cli/local)");

    Ok(())
}

/// Apply pre-configured identity links from the config file.
///
/// For each `[[identity.links]]` entry, resolves or creates the referenced
/// Astrid user and links the platform identity. Logs warnings on failure
/// but does not abort boot.
async fn apply_identity_config(
    store: &Arc<dyn astrid_storage::IdentityStore>,
    workspace_root: &std::path::Path,
) {
    let config = match astrid_config::Config::load(Some(workspace_root)) {
        Ok(resolved) => resolved.config,
        Err(e) => {
            tracing::debug!(error = %e, "No config loaded for identity links");
            return;
        },
    };

    for link_cfg in &config.identity.links {
        let result = apply_single_identity_link(store, link_cfg).await;
        if let Err(e) = result {
            tracing::warn!(
                platform = %link_cfg.platform,
                platform_user_id = %link_cfg.platform_user_id,
                astrid_user = %link_cfg.astrid_user,
                error = %e,
                "Failed to apply identity link from config"
            );
        }
    }
}

/// Apply a single identity link from config.
async fn apply_single_identity_link(
    store: &Arc<dyn astrid_storage::IdentityStore>,
    link_cfg: &astrid_config::types::IdentityLinkConfig,
) -> Result<(), astrid_storage::IdentityError> {
    // Resolve astrid_user: try UUID first, then name lookup, then create.
    let user_id = if let Ok(uuid) = uuid::Uuid::parse_str(&link_cfg.astrid_user) {
        // Ensure user record exists. If the UUID was explicitly specified in
        // config but doesn't exist in the store, that's a configuration error
        // - don't silently create a different user.
        if store.get_user(uuid).await?.is_none() {
            return Err(astrid_storage::IdentityError::UserNotFound(uuid));
        }
        uuid
    } else {
        // Try name lookup.
        if let Some(user) = store.get_user_by_name(&link_cfg.astrid_user).await? {
            user.id
        } else {
            let user = store.create_user(Some(&link_cfg.astrid_user)).await?;
            tracing::info!(
                user_id = %user.id,
                name = %link_cfg.astrid_user,
                "Created user from config identity link"
            );
            user.id
        }
    };

    let method = if link_cfg.method.is_empty() {
        "admin"
    } else {
        &link_cfg.method
    };

    // Check if link already points to the correct user - skip if idempotent.
    if let Some(existing) = store
        .resolve(&link_cfg.platform, &link_cfg.platform_user_id)
        .await?
        && existing.id == user_id
    {
        tracing::debug!(
            platform = %link_cfg.platform,
            platform_user_id = %link_cfg.platform_user_id,
            user_id = %user_id,
            "Identity link from config already exists"
        );
        return Ok(());
    }

    store
        .link(
            &link_cfg.platform,
            &link_cfg.platform_user_id,
            user_id,
            method,
        )
        .await?;

    tracing::info!(
        platform = %link_cfg.platform,
        platform_user_id = %link_cfg.platform_user_id,
        user_id = %user_id,
        "Applied identity link from config"
    );

    Ok(())
}
