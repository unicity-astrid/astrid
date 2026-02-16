//! Daemon `WebSocket` server.
//!
//! Implements the `jsonrpsee` server that listens on `127.0.0.1:{port}` and serves
//! the [`AstridRpc`] API. CLI clients connect via `WebSocket`.
//!
//! # Locking Design
//!
//! The runtime is stored behind a standalone `Arc` (immutable reference, never locked).
//! Sessions live in per-session `Mutex<AgentSession>` behind a shared session map.
//! The session map itself uses an `RwLock` but only for brief insert/remove/lookup —
//! never held across async operations like LLM calls or approval waits.
//!
//! This prevents the deadlock where `send_input` (holding a write lock during an
//! LLM turn) blocks `approval_response` (needing a read lock to deliver the
//! approval that the turn is waiting for).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use astrid_approval::allowance::Allowance;
use astrid_approval::budget::{WorkspaceBudgetSnapshot, WorkspaceBudgetTracker};
use astrid_audit::AuditLog;
use astrid_capabilities::CapabilityStore;
use astrid_core::{ApprovalDecision, ElicitationResponse, SessionId};
use astrid_crypto::KeyPair;
use astrid_hooks::{HookManager, discover_hooks};
use astrid_llm::{ClaudeProvider, LlmProvider, OpenAiCompatProvider, ZaiProvider};
use astrid_mcp::McpClient;
use astrid_plugins::{
    PluginContext, PluginId, PluginRegistry, PluginState, WasmPluginLoader, discover_manifests,
    manifest::PluginEntryPoint,
};
use astrid_runtime::{AgentRuntime, AgentSession, SessionStore, config_bridge};
use astrid_storage::{KvStore, ScopedKvStore, SurrealKvStore};
use chrono::{DateTime, Utc};
use jsonrpsee::server::{Server, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{PendingSubscriptionSink, SubscriptionMessage};
use tokio::sync::{Mutex, RwLock, broadcast};
use tracing::{debug, info, warn};

use crate::daemon_frontend::DaemonFrontend;
use crate::rpc::{
    AllowanceInfo, AstridRpcServer, AuditEntryInfo, BudgetInfo, DaemonEvent, DaemonStatus,
    McpServerInfo, PluginInfo, SessionInfo, ToolInfo, error_codes,
};

/// Build a workspace-namespaced key for the KV store.
///
/// Uses the workspace UUID to namespace keys, e.g. `ws:<uuid>:allowances`.
fn ws_ns(workspace_id: &uuid::Uuid, suffix: &str) -> String {
    format!("ws:{workspace_id}:{suffix}")
}

/// Shared context passed to `handle_watcher_reload` to avoid exceeding the
/// clippy `too_many_arguments` limit.
struct WatcherReloadContext {
    plugin_registry: Arc<RwLock<PluginRegistry>>,
    workspace_kv: Arc<dyn KvStore>,
    sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    mcp_client: McpClient,
    workspace_root: PathBuf,
    user_unloaded: Arc<RwLock<HashSet<PluginId>>>,
    wasm_loader: Arc<WasmPluginLoader>,
}

/// Guard that aborts a spawned Tokio task when dropped.
///
/// Unlike `JoinHandle::drop`, which does NOT cancel the task, this guard
/// ensures background tasks are cleaned up when their owner is cancelled.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Paths for daemon state files.
pub struct DaemonPaths {
    /// Directory for daemon files (e.g. `~/.astrid/`).
    pub base_dir: PathBuf,
}

impl DaemonPaths {
    /// Create paths for the default location using `AstridHome`.
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be resolved.
    pub fn default_dir() -> Result<Self, std::io::Error> {
        let home = astrid_core::dirs::AstridHome::resolve()?;
        Ok(Self {
            base_dir: home.root().to_path_buf(),
        })
    }

    /// PID file path.
    #[must_use]
    pub fn pid_file(&self) -> PathBuf {
        self.base_dir.join("daemon.pid")
    }

    /// Port file path (written on startup so CLI knows where to connect).
    #[must_use]
    pub fn port_file(&self) -> PathBuf {
        self.base_dir.join("daemon.port")
    }

    /// Daemon log file path (stderr is redirected here on auto-start).
    #[must_use]
    pub fn log_file(&self) -> PathBuf {
        self.base_dir.join("logs").join("daemon.log")
    }

    /// Mode file path (records whether daemon is ephemeral or persistent).
    #[must_use]
    pub fn mode_file(&self) -> PathBuf {
        self.base_dir.join("daemon.mode")
    }
}

/// Handle to a live session's shared state.
///
/// All fields are `Arc`-wrapped so `SessionHandle` is cheaply cloneable.
/// The `AgentSession` is behind a per-session `Mutex` so each session can
/// run independently without blocking the entire daemon.
#[derive(Clone)]
struct SessionHandle {
    /// The agent session (per-session lock — only locked during a turn).
    session: Arc<Mutex<AgentSession>>,
    /// The daemon frontend for this session (bridges Frontend trait to IPC).
    frontend: Arc<DaemonFrontend>,
    /// Broadcast channel for events going to CLI subscribers.
    event_tx: broadcast::Sender<DaemonEvent>,
    /// The workspace path for this session (if any).
    workspace: Option<PathBuf>,
    /// When the session was created (immutable).
    created_at: DateTime<Utc>,
    /// Handle to the currently running turn task (if any).
    turn_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

/// Options controlling daemon startup behaviour.
#[derive(Debug, Clone, Default)]
pub struct DaemonStartOptions {
    /// When `true`, the daemon shuts down automatically after all clients
    /// disconnect and the grace period elapses.
    pub ephemeral: bool,
    /// Override for the idle-shutdown grace period (seconds). Falls back to
    /// `gateway.idle_shutdown_secs` from the config.
    pub grace_period_secs: Option<u64>,
}

/// The daemon `WebSocket` server.
pub struct DaemonServer {
    /// The agent runtime (shared, immutable reference).
    runtime: Arc<AgentRuntime<Box<dyn LlmProvider>>>,
    /// Session map (brief locks only for insert/remove/lookup).
    sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    /// Plugin registry (shared across RPC handlers).
    plugin_registry: Arc<RwLock<PluginRegistry>>,
    /// Workspace KV store (used for plugin scoped storage on reload).
    workspace_kv: Arc<dyn KvStore>,
    /// MCP client (used to re-create MCP plugins on reload).
    mcp_client: McpClient,
    /// WASM plugin loader (shared configuration for reload consistency).
    wasm_loader: Arc<WasmPluginLoader>,
    /// Home directory for plugin paths.
    home: astrid_core::dirs::AstridHome,
    /// Workspace root directory.
    workspace_root: PathBuf,
    /// When the daemon started.
    #[allow(dead_code)]
    started_at: Instant,
    /// Shutdown signal.
    shutdown_tx: broadcast::Sender<()>,
    /// Filesystem paths for PID/port files.
    paths: DaemonPaths,
    /// Interval between health checks (from config, floored at 5s).
    health_interval: Duration,
    /// Whether this daemon is running in ephemeral mode.
    ephemeral: bool,
    /// Grace period before ephemeral shutdown (seconds).
    ephemeral_grace_secs: u64,
    /// Number of active `WebSocket` connections (event subscribers).
    active_connections: Arc<AtomicUsize>,
    /// Interval between session cleanup sweeps.
    session_cleanup_interval: Duration,
    /// Plugin IDs explicitly unloaded by the user via RPC.
    ///
    /// The watcher skips these to avoid re-loading plugins the user
    /// intentionally stopped. Cleared when the user re-loads via RPC.
    user_unloaded_plugins: Arc<RwLock<HashSet<PluginId>>>,
}

impl DaemonServer {
    /// Create and start a new daemon server.
    ///
    /// Binds to `127.0.0.1:0` (OS picks a free port), writes port/PID files,
    /// and returns the server handle for lifecycle management.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot bind or required components fail to initialize.
    #[allow(clippy::too_many_lines)]
    pub async fn start(
        options: DaemonStartOptions,
    ) -> Result<(Self, ServerHandle, SocketAddr, astrid_config::Config), crate::GatewayError> {
        let paths = DaemonPaths::default_dir().map_err(|e| {
            crate::GatewayError::Runtime(format!("Failed to resolve daemon paths: {e}"))
        })?;

        // Resolve and ensure directory structures.
        let home = astrid_core::dirs::AstridHome::resolve().map_err(|e| {
            crate::GatewayError::Runtime(format!("Failed to resolve home directory: {e}"))
        })?;
        home.ensure().map_err(|e| {
            crate::GatewayError::Runtime(format!(
                "Failed to create home directory {}: {e}",
                home.root().display()
            ))
        })?;

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let ws = astrid_core::dirs::WorkspaceDir::detect(&cwd);
        // Ensure workspace dir and generate workspace ID (idempotent).
        ws.ensure().map_err(|e| {
            crate::GatewayError::Runtime(format!(
                "Failed to ensure workspace directory {}: {e}",
                ws.root().display()
            ))
        })?;
        let workspace_id = ws.workspace_id().map_err(|e| {
            crate::GatewayError::Runtime(format!("Failed to get workspace ID: {e}"))
        })?;

        // Load unified configuration.
        let cfg = match astrid_config::Config::load(Some(&cwd)) {
            Ok(r) => r.config,
            Err(e) => {
                warn!(error = %e, "Failed to load config; falling back to defaults");
                astrid_config::Config::default()
            },
        };

        // Create LLM provider via config bridge (api key comes from config
        // precedence chain — no redundant env var read needed here).
        // If no key is configured yet, the provider starts with an empty key
        // and returns a clear error on the first actual LLM call.
        let provider_config = config_bridge::to_provider_config(&cfg);
        let llm: Box<dyn LlmProvider> = match cfg.model.provider.as_str() {
            "zai" => Box::new(ZaiProvider::new(provider_config)),
            "openai" | "openai-compat" => {
                let mut p = OpenAiCompatProvider::custom(
                    provider_config
                        .base_url
                        .as_deref()
                        .unwrap_or("https://api.openai.com/v1/chat/completions"),
                    Some(&provider_config.api_key),
                    &provider_config.model,
                )
                .with_max_tokens(provider_config.max_tokens)
                .with_temperature(provider_config.temperature);
                if let Some(ctx) = provider_config.context_window {
                    p = p.with_max_context(ctx);
                }
                Box::new(p)
            },
            // Default to Claude.
            _ => Box::new(ClaudeProvider::new(provider_config)),
        };

        // Load MCP server definitions from unified config.
        let servers_config = config_bridge::to_servers_config(&cfg);
        let mcp = McpClient::with_config(servers_config);

        // Auto-connect configured auto-start servers.
        match mcp.connect_auto_servers().await {
            Ok(n) if n > 0 => info!(count = n, "Auto-connected MCP servers"),
            Ok(_) => info!("No auto-start MCP servers configured"),
            Err(e) => warn!(error = %e, "Error during MCP auto-connect"),
        }

        // Load key once, get two independent instances (avoids double disk read
        // and double un-zeroized intermediate buffer).
        let (audit_key, key) =
            KeyPair::load_or_generate_pair(home.user_key_path()).map_err(|e| {
                crate::GatewayError::Runtime(format!("Failed to load/generate key: {e}"))
            })?;

        let audit = AuditLog::open(home.audit_db_path(), audit_key)
            .map_err(|e| crate::GatewayError::Runtime(format!("Failed to open audit log: {e}")))?;

        let sessions = SessionStore::from_home(&home);

        // Convert workspace and runtime config via bridge.
        let config = config_bridge::to_runtime_config(&cfg, &cwd);

        let hook_manager = HookManager::new();
        let hooks_extra = vec![home.hooks_dir()];
        let discovered = discover_hooks(Some(&hooks_extra));
        hook_manager.register_all(discovered).await;

        // Clone MCP client for plugin registry before moving into runtime.
        let mcp_for_plugins = mcp.clone();
        // Clone MCP client for watcher-driven plugin reloads.
        let mcp_for_watcher = mcp.clone();

        let runtime =
            AgentRuntime::new_arc(llm, mcp, audit, sessions, key, config, Some(hook_manager));

        // Open persistent capability store (tokens survive restarts).
        let capabilities_store = Arc::new(
            CapabilityStore::with_persistence(home.capabilities_db_path()).map_err(|e| {
                crate::GatewayError::Runtime(format!("Failed to open capabilities store: {e}"))
            })?,
        );

        // Open workspace state KV store (allowances, budget, escape state).
        // Lives in ~/.astrid/state.db, namespaced by workspace ID.
        let workspace_kv: Arc<dyn KvStore> =
            Arc::new(SurrealKvStore::open(home.state_db_path()).map_err(|e| {
                crate::GatewayError::Runtime(format!("Failed to open workspace state store: {e}"))
            })?);

        // Open persistent KV store for deferred resolution queue.
        let deferred_kv: Arc<dyn KvStore> =
            Arc::new(SurrealKvStore::open(home.deferred_db_path()).map_err(|e| {
                crate::GatewayError::Runtime(format!("Failed to open deferred store: {e}"))
            })?);

        // Discover and register plugins (does not load them yet).
        let mut plugin_registry = PluginRegistry::new();
        let wasm_loader = Arc::new(WasmPluginLoader::new());
        let plugin_dirs = vec![home.plugins_dir()];
        let discovered = discover_manifests(Some(&plugin_dirs));
        for (mut manifest, plugin_dir) in discovered {
            // Resolve relative WASM paths to absolute, anchored at the
            // directory where the manifest was discovered. Without this,
            // WasmPlugin::do_load would resolve them against the daemon CWD
            // which is wrong for both user-level and workspace-level plugins.
            if let PluginEntryPoint::Wasm { ref mut path, .. } = manifest.entry_point
                && path.is_relative()
            {
                *path = plugin_dir.join(&*path);
            }

            let plugin: Box<dyn astrid_plugins::Plugin> = match &manifest.entry_point {
                PluginEntryPoint::Wasm { .. } => {
                    Box::new(wasm_loader.create_plugin(manifest.clone()))
                },
                PluginEntryPoint::Mcp { .. } => {
                    match astrid_plugins::create_plugin(
                        manifest.clone(),
                        Some(mcp_for_plugins.clone()),
                        Some(plugin_dir.clone()),
                    ) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(plugin = %manifest.id, error = %e, "Failed to create MCP plugin");
                            continue;
                        },
                    }
                },
            };
            if let Err(e) = plugin_registry.register(plugin) {
                warn!(plugin = %manifest.id, error = %e, "Failed to register plugin");
            }
        }
        let plugin_registry = Arc::new(RwLock::new(plugin_registry));

        let (shutdown_tx, _) = broadcast::channel(1);
        let session_map: Arc<RwLock<HashMap<SessionId, SessionHandle>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Build and start the jsonrpsee server.
        let server = Server::builder()
            .build("127.0.0.1:0")
            .await
            .map_err(|e| crate::GatewayError::Runtime(format!("Failed to bind server: {e}")))?;

        let addr = server
            .local_addr()
            .map_err(|e| crate::GatewayError::Runtime(format!("Failed to get address: {e}")))?;

        // Load workspace budget from KV store and construct the tracker.
        let ws_max_usd = config_bridge::workspace_max_usd(&cfg);
        let ws_warn_pct = cfg.budget.warn_at_percent;
        let ns_budget = ws_ns(&workspace_id, "budget");
        let workspace_budget_tracker = match workspace_kv.get(&ns_budget, "all").await {
            Ok(Some(data)) => {
                if let Ok(snapshot) = serde_json::from_slice::<WorkspaceBudgetSnapshot>(&data) {
                    Arc::new(WorkspaceBudgetTracker::restore(
                        &snapshot,
                        ws_max_usd,
                        ws_warn_pct,
                    ))
                } else {
                    Arc::new(WorkspaceBudgetTracker::new(ws_max_usd, ws_warn_pct))
                }
            },
            _ => Arc::new(WorkspaceBudgetTracker::new(ws_max_usd, ws_warn_pct)),
        };

        let model_name = cfg.model.model.clone();
        let active_connections = Arc::new(AtomicUsize::new(0));
        let user_unloaded_plugins: Arc<RwLock<HashSet<PluginId>>> =
            Arc::new(RwLock::new(HashSet::new()));

        let rpc_impl = RpcImpl {
            runtime: Arc::clone(&runtime),
            sessions: Arc::clone(&session_map),
            plugin_registry: Arc::clone(&plugin_registry),
            deferred_kv,
            capabilities_store,
            workspace_kv: Arc::clone(&workspace_kv),
            workspace_budget_tracker,
            started_at: Instant::now(),
            shutdown_tx: shutdown_tx.clone(),
            workspace_id,
            model_name,
            active_connections: Arc::clone(&active_connections),
            ephemeral: options.ephemeral,
            user_unloaded_plugins: Arc::clone(&user_unloaded_plugins),
            workspace_root: cwd.clone(),
        };

        let handle = server.start(rpc_impl.into_rpc());

        // Write PID and port files.
        let pid = std::process::id();
        std::fs::write(paths.pid_file(), pid.to_string())
            .map_err(|e| crate::GatewayError::Runtime(format!("Failed to write PID file: {e}")))?;
        std::fs::write(paths.port_file(), addr.port().to_string())
            .map_err(|e| crate::GatewayError::Runtime(format!("Failed to write port file: {e}")))?;

        info!(addr = %addr, pid = pid, "Daemon server started");

        // Background task: auto-load discovered plugins after the server is
        // accepting connections (avoids blocking the 5s CLI connect timeout).
        // Each plugin is taken out of the registry, loaded without the lock
        // held, then put back — so MCP handshakes don't block other operations.
        {
            let registry_clone = Arc::clone(&plugin_registry);
            let kv_clone = Arc::clone(&workspace_kv);
            let workspace_root = cwd.clone();
            tokio::spawn(async move {
                // Collect IDs under a brief read lock.
                let plugin_ids: Vec<PluginId> = {
                    let registry = registry_clone.read().await;
                    registry.list().into_iter().cloned().collect()
                };

                for plugin_id in plugin_ids {
                    // Take the plugin out (brief write lock).
                    let mut plugin = {
                        let mut registry = registry_clone.write().await;
                        match registry.unregister(&plugin_id) {
                            Ok(p) => p,
                            Err(_) => continue,
                        }
                    };

                    let kv = match ScopedKvStore::new(
                        Arc::clone(&kv_clone),
                        format!("plugin:{plugin_id}"),
                    ) {
                        Ok(kv) => kv,
                        Err(e) => {
                            warn!(plugin_id = %plugin_id, error = %e, "Failed to create plugin KV scope");
                            // Put it back.
                            let mut registry = registry_clone.write().await;
                            let _ = registry.register(plugin);
                            continue;
                        },
                    };
                    let config = plugin.manifest().config.clone();
                    let ctx = PluginContext::new(workspace_root.clone(), kv, config);

                    // Load without holding any lock.
                    if let Err(e) = plugin.load(&ctx).await {
                        warn!(plugin_id = %plugin_id, error = %e, "Failed to auto-load plugin");
                    } else {
                        info!(plugin_id = %plugin_id, "Auto-loaded plugin");
                    }

                    // Put the plugin back (brief write lock).
                    let mut registry = registry_clone.write().await;
                    let _ = registry.register(plugin);
                }
            });
        }

        // Floor health interval at 5s to prevent zero/tiny intervals.
        let health_interval = Duration::from_secs(cfg.gateway.health_interval_secs.max(5));

        // Resolve ephemeral grace period: CLI flag → config default.
        let ephemeral_grace_secs = options
            .grace_period_secs
            .unwrap_or(cfg.gateway.idle_shutdown_secs);

        let session_cleanup_interval =
            Duration::from_secs(cfg.gateway.session_cleanup_interval_secs.max(10));

        // Write mode file so callers (e.g. `daemon status`) can read it.
        let mode_str = if options.ephemeral {
            "ephemeral"
        } else {
            "persistent"
        };
        let _ = std::fs::write(paths.mode_file(), mode_str);

        let daemon = Self {
            runtime,
            sessions: session_map,
            plugin_registry,
            workspace_kv,
            mcp_client: mcp_for_watcher,
            wasm_loader,
            home: home.clone(),
            workspace_root: cwd,
            started_at: Instant::now(),
            shutdown_tx,
            paths,
            health_interval,
            ephemeral: options.ephemeral,
            ephemeral_grace_secs,
            active_connections,
            session_cleanup_interval,
            user_unloaded_plugins,
        };
        Ok((daemon, handle, addr, cfg))
    }

    /// Gracefully unload all registered plugins.
    pub async fn shutdown_plugins(&self) {
        let mut registry = self.plugin_registry.write().await;
        let ids: Vec<PluginId> = registry.list().into_iter().cloned().collect();
        for id in ids {
            if let Some(plugin) = registry.get_mut(&id)
                && let Err(e) = plugin.unload().await
            {
                warn!(plugin_id = %id, error = %e, "Error unloading plugin during shutdown");
            }
        }
        info!("Plugins shut down");
    }

    /// Gracefully shut down all MCP servers.
    pub async fn shutdown_servers(&self) {
        if let Err(e) = self.runtime.mcp().shutdown().await {
            warn!(error = %e, "Error shutting down MCP servers");
        }
        info!("MCP servers shut down");
    }

    /// Spawn the health monitoring loop.
    ///
    /// Checks server health at the configured interval (from
    /// `gateway.health_interval_secs`, floored at 5 s). Dead servers with a
    /// restart policy are automatically reconnected.
    ///
    /// The loop clones the `McpClient` out of the runtime once at startup
    /// (cheap — all `Arc` internals) so that health checks and reconnects
    /// never block session-mutating RPCs.
    #[must_use]
    pub fn spawn_health_loop(&self) -> tokio::task::JoinHandle<()> {
        let mcp = self.runtime.mcp().clone();
        let health_interval = self.health_interval;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(health_interval);
            loop {
                interval.tick().await;

                let health = mcp.server_manager().health_check().await;

                for (name, alive) in &health {
                    if !alive {
                        warn!(server = %name, "MCP server is dead");
                        match mcp.try_reconnect(name).await {
                            Ok(true) => {
                                info!(server = %name, "Server restarted by health loop");
                            },
                            Ok(false) => {
                                info!(server = %name, "Restart not allowed by policy");
                            },
                            Err(e) => {
                                warn!(server = %name, error = %e, "Restart failed");
                            },
                        }
                    }
                }
            }
        })
    }

    /// Spawn the ephemeral shutdown monitor.
    ///
    /// Returns `None` if the daemon is in persistent mode. In ephemeral mode,
    /// the monitor waits an initial 10 s for the first client to connect, then
    /// polls `active_connections` every 5 s. When all connections have been
    /// gone for `ephemeral_grace_secs` it sends a shutdown signal.
    #[must_use]
    pub fn spawn_ephemeral_monitor(&self) -> Option<tokio::task::JoinHandle<()>> {
        if !self.ephemeral {
            return None;
        }

        let connections = Arc::clone(&self.active_connections);
        let shutdown_tx = self.shutdown_tx.clone();
        let grace = Duration::from_secs(self.ephemeral_grace_secs);

        Some(tokio::spawn(async move {
            // Give the first client time to connect after auto-start.
            tokio::time::sleep(Duration::from_secs(10)).await;

            let mut idle_since: Option<Instant> = None;

            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;

                let count = connections.load(Ordering::Relaxed);
                if count == 0 {
                    let start = *idle_since.get_or_insert_with(Instant::now);
                    if start.elapsed() >= grace {
                        info!(
                            "Ephemeral daemon idle for {}s — shutting down",
                            grace.as_secs()
                        );
                        let _ = shutdown_tx.send(());
                        return;
                    }
                } else {
                    // Reset whenever at least one client is connected.
                    idle_since = None;
                }
            }
        }))
    }

    /// Spawn the stale-session cleanup loop.
    ///
    /// Periodically sweeps the session map looking for orphaned sessions
    /// (no event subscribers and no active turn). Orphaned sessions are
    /// saved to disk and removed from the in-memory map.
    #[must_use]
    pub fn spawn_session_cleanup_loop(&self) -> tokio::task::JoinHandle<()> {
        let sessions = Arc::clone(&self.sessions);
        let runtime = Arc::clone(&self.runtime);
        let interval = self.session_cleanup_interval;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;

                let orphaned: Vec<SessionId> = {
                    let map = sessions.read().await;
                    let mut ids = Vec::new();
                    for (id, handle) in map.iter() {
                        let no_subscribers = handle.event_tx.receiver_count() == 0;
                        let no_active_turn = handle.turn_handle.lock().await.is_none();
                        if no_subscribers && no_active_turn {
                            ids.push(id.clone());
                        }
                    }
                    ids
                };

                if orphaned.is_empty() {
                    continue;
                }

                let mut map = sessions.write().await;
                for id in &orphaned {
                    if let Some(handle) = map.remove(id) {
                        let session = handle.session.lock().await;
                        if let Err(e) = runtime.save_session(&session) {
                            warn!(session_id = %id, error = %e, "Failed to save orphaned session");
                        } else {
                            info!(session_id = %id, "Cleaned up orphaned session");
                        }
                    }
                }
            }
        })
    }

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
        // watching each directory — no need to log here too.

        let reload_ctx = WatcherReloadContext {
            plugin_registry: Arc::clone(&self.plugin_registry),
            workspace_kv: Arc::clone(&self.workspace_kv),
            sessions: Arc::clone(&self.sessions),
            mcp_client: self.mcp_client.clone(),
            workspace_root: self.workspace_root.clone(),
            user_unloaded: Arc::clone(&self.user_unloaded_plugins),
            wasm_loader: Arc::clone(&self.wasm_loader),
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
            // _guard dropped here → inner watcher task is aborted.
        });

        Some(handle)
    }

    /// Handle a single plugin reload triggered by the file watcher.
    ///
    /// Discovers the manifest in the changed directory, unloads the old plugin
    /// if loaded, re-registers it, loads it with a fresh context, and broadcasts
    /// the result to all connected sessions.
    async fn handle_watcher_reload(plugin_dir: &std::path::Path, ctx: &WatcherReloadContext) {
        let WatcherReloadContext {
            plugin_registry,
            workspace_kv,
            sessions,
            mcp_client,
            workspace_root,
            user_unloaded,
            wasm_loader,
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

        // Unload → unregister → re-register → load.
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
    fn create_plugin_from_manifest(
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
    async fn swap_and_load_plugin(
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
    async fn broadcast_event(
        sessions: &Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
        event: DaemonEvent,
    ) {
        let map = sessions.read().await;
        for handle in map.values() {
            let _ = handle.event_tx.send(event.clone());
        }
    }

    /// Whether this daemon is running in ephemeral mode.
    #[must_use]
    pub fn is_ephemeral(&self) -> bool {
        self.ephemeral
    }

    /// Subscribe to the shutdown signal.
    ///
    /// The returned receiver fires when an RPC `shutdown()` call is made.
    /// Use with `tokio::select!` alongside `ctrl_c()` in the daemon's
    /// foreground loop.
    #[must_use]
    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Clean up daemon files (PID, port, mode) on shutdown.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(self.paths.pid_file());
        let _ = std::fs::remove_file(self.paths.port_file());
        let _ = std::fs::remove_file(self.paths.mode_file());
        info!("Daemon files cleaned up");
    }

    /// Read the port from the port file (used by CLI to find the daemon).
    #[must_use]
    pub fn read_port(paths: &DaemonPaths) -> Option<u16> {
        std::fs::read_to_string(paths.port_file())
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    /// Read the PID from the PID file.
    #[must_use]
    pub fn read_pid(paths: &DaemonPaths) -> Option<u32> {
        std::fs::read_to_string(paths.pid_file())
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    /// Check if a daemon is running (PID file exists and process is alive).
    #[must_use]
    pub fn is_running(paths: &DaemonPaths) -> bool {
        if let Some(pid) = Self::read_pid(paths) {
            is_process_alive(pid)
        } else {
            false
        }
    }
}

/// Check if a process with the given PID is alive.
fn is_process_alive(pid: u32) -> bool {
    // Use `kill -0 <pid>` to check if the process exists.
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

// ---------- RPC Implementation ----------

/// The jsonrpsee RPC method handler.
///
/// Uses per-session locking to avoid the deadlock where `send_input`
/// (running an LLM turn) blocks `approval_response` (delivering the
/// approval that the turn is waiting for).
struct RpcImpl {
    /// The agent runtime (immutable, never locked).
    runtime: Arc<AgentRuntime<Box<dyn LlmProvider>>>,
    /// Session map (brief locks for insert/remove/lookup only).
    sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    /// Plugin registry (shared, behind `RwLock`).
    plugin_registry: Arc<RwLock<PluginRegistry>>,
    /// Shared KV store for deferred resolution persistence.
    deferred_kv: Arc<dyn KvStore>,
    /// Shared persistent capability store (tokens survive restarts).
    capabilities_store: Arc<CapabilityStore>,
    /// Shared workspace state KV store (allowances, budget, escape).
    workspace_kv: Arc<dyn KvStore>,
    /// Workspace cumulative budget tracker (shared across sessions).
    workspace_budget_tracker: Arc<WorkspaceBudgetTracker>,
    /// When the daemon started.
    started_at: Instant,
    /// Shutdown signal.
    shutdown_tx: broadcast::Sender<()>,
    /// Workspace UUID for namespacing KV keys.
    workspace_id: uuid::Uuid,
    /// Model name from config (set on sessions).
    model_name: String,
    /// Number of active `WebSocket` connections (event subscribers).
    active_connections: Arc<AtomicUsize>,
    /// Whether the daemon is running in ephemeral mode.
    ephemeral: bool,
    /// Plugin IDs explicitly unloaded by the user (shared with watcher).
    user_unloaded_plugins: Arc<RwLock<HashSet<PluginId>>>,
    /// Workspace root directory (consistent with watcher reload path).
    workspace_root: PathBuf,
}

#[jsonrpsee::core::async_trait]
impl AstridRpcServer for RpcImpl {
    async fn create_session(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<SessionInfo, ErrorObjectOwned> {
        let mut session = self.runtime.create_session(workspace_path.as_deref());
        session.workspace_path = workspace_path.clone();
        session.model = Some(self.model_name.clone());

        // Wire persistent capability store (shared across sessions).
        let session = session.with_capability_store(Arc::clone(&self.capabilities_store));

        // Wire persistent deferred resolution queue (per-session namespace).
        let scoped = ScopedKvStore::new(
            Arc::clone(&self.deferred_kv),
            format!("deferred:{}", session.id.0),
        )
        .map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INTERNAL_ERROR,
                format!("Failed to create deferred store scope: {e}"),
                None::<()>,
            )
        })?;
        let session = session
            .with_persistent_deferred_queue(scoped)
            .await
            .map_err(|e| {
                ErrorObjectOwned::owned(
                    error_codes::INTERNAL_ERROR,
                    format!("Failed to initialize deferred queue: {e}"),
                    None::<()>,
                )
            })?;

        // Wire workspace cumulative budget tracker.
        let mut session = session.with_workspace_budget(Arc::clone(&self.workspace_budget_tracker));

        // Load workspace-scoped allowances (persisted across sessions).
        let ws_allowances = self.load_workspace_allowances().await;
        if !ws_allowances.is_empty() {
            session.import_workspace_allowances(ws_allowances);
        }

        // Load workspace escape cache (persisted "AllowAlways" paths).
        if let Some(state) = self.load_workspace_escape().await {
            session.escape_handler.restore_state(state);
        }

        let pending_deferred_count = session.approval_manager.get_pending_resolutions().len();

        let session_id = session.id.clone();
        let created_at = session.created_at;
        let message_count = session.messages.len();

        // Create a broadcast channel for this session's events.
        let (event_tx, _) = broadcast::channel(256);
        let frontend = Arc::new(DaemonFrontend::new(event_tx.clone()));

        let handle = SessionHandle {
            session: Arc::new(Mutex::new(session)),
            frontend,
            event_tx,
            workspace: workspace_path.clone(),
            created_at,
            turn_handle: Arc::new(Mutex::new(None)),
        };

        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), handle);
        }

        let info = SessionInfo {
            id: session_id.clone(),
            workspace: workspace_path,
            created_at,
            message_count,
            pending_deferred_count,
        };

        info!(session_id = %info.id, "Created new session via RPC");
        Ok(info)
    }

    async fn resume_session(&self, session_id: SessionId) -> Result<SessionInfo, ErrorObjectOwned> {
        // Check if already live (brief read lock).
        {
            let sessions = self.sessions.read().await;
            if let Some(handle) = sessions.get(&session_id) {
                let session = handle.session.lock().await;
                let pending_deferred_count =
                    session.approval_manager.get_pending_resolutions().len();
                return Ok(SessionInfo {
                    id: session_id,
                    workspace: handle.workspace.clone(),
                    created_at: handle.created_at,
                    message_count: session.messages.len(),
                    pending_deferred_count,
                });
            }
        }

        // Try to load from disk.
        let session = self
            .runtime
            .load_session(&session_id)
            .map_err(|e| {
                ErrorObjectOwned::owned(
                    error_codes::INTERNAL_ERROR,
                    format!("Failed to load session: {e}"),
                    None::<()>,
                )
            })?
            .ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?;

        // Wire persistent capability store for the resumed session.
        let session = session.with_capability_store(Arc::clone(&self.capabilities_store));

        // Wire persistent deferred resolution queue for the resumed session.
        let scoped = ScopedKvStore::new(
            Arc::clone(&self.deferred_kv),
            format!("deferred:{}", session.id.0),
        )
        .map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INTERNAL_ERROR,
                format!("Failed to create deferred store scope: {e}"),
                None::<()>,
            )
        })?;
        let session = session
            .with_persistent_deferred_queue(scoped)
            .await
            .map_err(|e| {
                ErrorObjectOwned::owned(
                    error_codes::INTERNAL_ERROR,
                    format!("Failed to initialize deferred queue: {e}"),
                    None::<()>,
                )
            })?;

        // Wire workspace cumulative budget tracker.
        let mut session = session.with_workspace_budget(Arc::clone(&self.workspace_budget_tracker));

        // Set the model name (may differ from saved value if config changed).
        session.model = Some(self.model_name.clone());

        // Load workspace-scoped allowances (persisted across sessions).
        let ws_allowances = self.load_workspace_allowances().await;
        if !ws_allowances.is_empty() {
            session.import_workspace_allowances(ws_allowances);
        }

        // Load workspace escape cache (persisted "AllowAlways" paths).
        if let Some(state) = self.load_workspace_escape().await {
            session.escape_handler.restore_state(state);
        }

        let pending_deferred_count = session.approval_manager.get_pending_resolutions().len();

        let workspace = session.workspace_path.clone();
        let created_at = session.created_at;
        let message_count = session.messages.len();

        let (event_tx, _) = broadcast::channel(256);
        let frontend = Arc::new(DaemonFrontend::new(event_tx.clone()));

        let handle = SessionHandle {
            session: Arc::new(Mutex::new(session)),
            frontend,
            event_tx,
            workspace: workspace.clone(),
            created_at,
            turn_handle: Arc::new(Mutex::new(None)),
        };

        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), handle);
        }

        Ok(SessionInfo {
            id: session_id,
            workspace,
            created_at,
            message_count,
            pending_deferred_count,
        })
    }

    async fn send_input(
        &self,
        session_id: SessionId,
        input: String,
    ) -> Result<(), ErrorObjectOwned> {
        // Look up the session handle (brief read lock on the map).
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };
        // Map lock released here.

        let runtime = Arc::clone(&self.runtime);
        let event_tx = handle.event_tx.clone();
        let frontend = Arc::clone(&handle.frontend);
        let session_mutex = Arc::clone(&handle.session);
        let workspace_kv = Arc::clone(&self.workspace_kv);
        let ws_budget_tracker = Arc::clone(&self.workspace_budget_tracker);
        let ws_id = self.workspace_id;
        let turn_handle = Arc::clone(&handle.turn_handle);

        // Run the agent turn in a background task.
        // Only the per-session mutex is held — other sessions, approval_response,
        // status, list_sessions, etc. all proceed without blocking.
        let join_handle = tokio::spawn(async move {
            let mut session = session_mutex.lock().await;

            let result = runtime
                .run_turn_streaming(&mut session, &input, Arc::clone(&frontend))
                .await;

            // Auto-save after every turn for crash recovery.
            if let Err(e) = runtime.save_session(&session) {
                warn!(error = %e, "Failed to auto-save session after turn");
            } else {
                let _ = event_tx.send(DaemonEvent::SessionSaved);
            }

            // Persist workspace-scoped allowances after each turn so that
            // "Allow Workspace" decisions survive daemon restarts.
            let ws_allowances = session.export_workspace_allowances();
            if !ws_allowances.is_empty()
                && let Ok(data) = serde_json::to_vec(&ws_allowances)
                && let Err(e) = workspace_kv
                    .set(&ws_ns(&ws_id, "allowances"), "all", data)
                    .await
            {
                warn!(error = %e, "Failed to save workspace allowances after turn");
            }

            // Persist workspace cumulative budget snapshot.
            {
                let snapshot = ws_budget_tracker.snapshot();
                if let Ok(data) = serde_json::to_vec(&snapshot)
                    && let Err(e) = workspace_kv
                        .set(&ws_ns(&ws_id, "budget"), "all", data)
                        .await
                {
                    warn!(error = %e, "Failed to save workspace budget after turn");
                }
            }

            // Persist workspace escape cache ("AllowAlways" paths).
            {
                let escape_state = session.escape_handler.export_state();
                if !escape_state.remembered_paths.is_empty()
                    && let Ok(data) = serde_json::to_vec(&escape_state)
                    && let Err(e) = workspace_kv
                        .set(&ws_ns(&ws_id, "escape"), "all", data)
                        .await
                {
                    warn!(error = %e, "Failed to save workspace escape state after turn");
                }
            }

            // Send context usage update before signalling turn complete.
            let _ = event_tx.send(DaemonEvent::Usage {
                context_tokens: session.token_count,
                max_context_tokens: runtime.config().max_context_tokens,
            });

            match result {
                Ok(()) => {
                    let _ = event_tx.send(DaemonEvent::TurnComplete);
                },
                Err(e) => {
                    let _ = event_tx.send(DaemonEvent::Error(e.to_string()));
                    let _ = event_tx.send(DaemonEvent::TurnComplete);
                },
            }

            // Clear the turn handle now that the turn is done.
            *turn_handle.lock().await = None;
        });

        // Store the join handle so cancel_turn can abort it.
        *handle.turn_handle.lock().await = Some(join_handle);

        Ok(())
    }

    async fn approval_response(
        &self,
        session_id: SessionId,
        request_id: String,
        decision: ApprovalDecision,
    ) -> Result<(), ErrorObjectOwned> {
        // Look up the session handle (brief read lock).
        // No session mutex needed — the frontend's pending_approvals has its own lock.
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        if !handle
            .frontend
            .resolve_approval(&request_id, decision)
            .await
        {
            return Err(ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("No pending approval with id: {request_id}"),
                None::<()>,
            ));
        }

        Ok(())
    }

    async fn elicitation_response(
        &self,
        session_id: SessionId,
        request_id: String,
        response: ElicitationResponse,
    ) -> Result<(), ErrorObjectOwned> {
        // Look up the session handle (brief read lock).
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        if !handle
            .frontend
            .resolve_elicitation(&request_id, response)
            .await
        {
            return Err(ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("No pending elicitation with id: {request_id}"),
                None::<()>,
            ));
        }

        Ok(())
    }

    async fn list_sessions(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<Vec<SessionInfo>, ErrorObjectOwned> {
        let sessions = self.sessions.read().await;
        let mut result = Vec::new();

        for (id, handle) in sessions.iter() {
            // Filter by workspace path if provided.
            if let Some(ref ws) = workspace_path
                && handle.workspace.as_ref() != Some(ws)
            {
                continue;
            }

            let session = handle.session.lock().await;
            let pending_deferred_count = session.approval_manager.get_pending_resolutions().len();
            result.push(SessionInfo {
                id: id.clone(),
                workspace: handle.workspace.clone(),
                created_at: handle.created_at,
                message_count: session.messages.len(),
                pending_deferred_count,
            });
        }

        Ok(result)
    }

    async fn end_session(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned> {
        // Remove the session from the map (brief write lock).
        let handle = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(&session_id).ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        // Lock the session to export, clear, and save.
        let session = handle.session.lock().await;

        // Persist workspace-scoped allowances before clearing session state.
        let ws_allowances = session.export_workspace_allowances();
        if !ws_allowances.is_empty() {
            self.save_workspace_allowances(&ws_allowances).await;
        }

        // Persist workspace cumulative budget snapshot.
        self.save_workspace_budget().await;

        // Persist workspace escape cache.
        let escape_state = session.escape_handler.export_state();
        if !escape_state.remembered_paths.is_empty() {
            self.save_workspace_escape(&escape_state).await;
        }

        // Clear session allowances (security hygiene).
        session.allowance_store.clear_session_allowances();

        // Save session before ending.
        if let Err(e) = self.runtime.save_session(&session) {
            warn!(session_id = %session_id, error = %e, "Failed to save session on end");
        }

        info!(session_id = %session_id, "Session ended via RPC");
        Ok(())
    }

    async fn status(&self) -> Result<DaemonStatus, ErrorObjectOwned> {
        let mcp = self.runtime.mcp();
        let session_count = self.sessions.read().await.len();

        let plugins_loaded = {
            let registry = self.plugin_registry.read().await;
            registry
                .list()
                .iter()
                .filter(|id| {
                    registry
                        .get(id)
                        .is_some_and(|p| matches!(p.state(), PluginState::Ready))
                })
                .count()
        };

        Ok(DaemonStatus {
            running: true,
            uptime_secs: self.started_at.elapsed().as_secs(),
            active_sessions: session_count,
            version: env!("CARGO_PKG_VERSION").to_string(),
            mcp_servers_configured: mcp.server_manager().configured_count(),
            mcp_servers_running: mcp.server_manager().running_count().await,
            plugins_loaded,
            ephemeral: self.ephemeral,
            active_connections: self.active_connections.load(Ordering::Relaxed),
        })
    }

    async fn list_servers(&self) -> Result<Vec<McpServerInfo>, ErrorObjectOwned> {
        let statuses = self.runtime.mcp().server_statuses().await;
        Ok(statuses
            .into_iter()
            .map(|s| McpServerInfo {
                name: s.name,
                alive: s.alive,
                ready: s.ready,
                tool_count: s.tool_count,
                restart_count: s.restart_count,
                description: s.description,
            })
            .collect())
    }

    async fn start_server(&self, name: String) -> Result<(), ErrorObjectOwned> {
        self.runtime.mcp().connect(&name).await.map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INTERNAL_ERROR,
                format!("Failed to start server {name}: {e}"),
                None::<()>,
            )
        })?;
        info!(server = %name, "Server started via RPC");
        Ok(())
    }

    async fn stop_server(&self, name: String) -> Result<(), ErrorObjectOwned> {
        self.runtime.mcp().disconnect(&name).await.map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INTERNAL_ERROR,
                format!("Failed to stop server {name}: {e}"),
                None::<()>,
            )
        })?;
        info!(server = %name, "Server stopped via RPC");
        Ok(())
    }

    async fn list_tools(&self) -> Result<Vec<ToolInfo>, ErrorObjectOwned> {
        let mut result: Vec<ToolInfo> = Vec::new();

        // MCP server tools.
        let tools = self.runtime.mcp().list_tools().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INTERNAL_ERROR,
                format!("Failed to list tools: {e}"),
                None::<()>,
            )
        })?;
        result.extend(tools.into_iter().map(|t| ToolInfo {
            name: t.name,
            server: t.server,
            description: t.description,
        }));

        // Plugin tools.
        let registry = self.plugin_registry.read().await;
        for td in registry.all_tool_definitions() {
            // Qualified name is "plugin:{plugin_id}:{tool_name}".
            // Extract the "plugin:{plugin_id}" prefix as the server field.
            let server = td
                .name
                .rsplit_once(':')
                .map_or_else(|| td.name.clone(), |(prefix, _)| prefix.to_string());
            result.push(ToolInfo {
                name: td.name,
                server,
                description: Some(td.description),
            });
        }

        Ok(result)
    }

    async fn shutdown(&self) -> Result<(), ErrorObjectOwned> {
        let _ = self.shutdown_tx.send(());
        info!("Shutdown requested via RPC");
        Ok(())
    }

    async fn session_budget(&self, session_id: SessionId) -> Result<BudgetInfo, ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        let session = handle.session.lock().await;
        let budget = &session.budget_tracker;

        let (workspace_spent, workspace_max, workspace_remaining) =
            if let Some(ref ws_budget) = session.workspace_budget_tracker {
                (
                    Some(ws_budget.spent()),
                    ws_budget.remaining().map(|r| r + ws_budget.spent()),
                    ws_budget.remaining(),
                )
            } else {
                (None, None, None)
            };

        Ok(BudgetInfo {
            session_spent_usd: budget.spent(),
            session_max_usd: budget.config().session_max_usd,
            session_remaining_usd: budget.remaining(),
            per_action_max_usd: budget.config().per_action_max_usd,
            warn_at_percent: budget.config().warn_at_percent,
            workspace_spent_usd: workspace_spent,
            workspace_max_usd: workspace_max,
            workspace_remaining_usd: workspace_remaining,
        })
    }

    async fn session_allowances(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<AllowanceInfo>, ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        let session = handle.session.lock().await;
        let mut infos = Vec::new();

        for allowance in session.allowance_store.export_session_allowances() {
            infos.push(AllowanceInfo {
                id: allowance.id.to_string(),
                pattern: format!("{:?}", allowance.action_pattern),
                session_only: allowance.session_only,
                created_at: DateTime::from(allowance.created_at),
                expires_at: allowance.expires_at.map(DateTime::from),
                uses_remaining: allowance.uses_remaining,
            });
        }

        for allowance in session.allowance_store.export_workspace_allowances() {
            infos.push(AllowanceInfo {
                id: allowance.id.to_string(),
                pattern: format!("{:?}", allowance.action_pattern),
                session_only: allowance.session_only,
                created_at: DateTime::from(allowance.created_at),
                expires_at: allowance.expires_at.map(DateTime::from),
                uses_remaining: allowance.uses_remaining,
            });
        }

        Ok(infos)
    }

    async fn session_audit(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<AuditEntryInfo>, ErrorObjectOwned> {
        let entries = self
            .runtime
            .audit()
            .get_session_entries(&session_id)
            .map_err(|e| {
                ErrorObjectOwned::owned(
                    error_codes::INTERNAL_ERROR,
                    format!("Failed to query audit log: {e}"),
                    None::<()>,
                )
            })?;

        let limit = limit.unwrap_or(20);
        let start = entries.len().saturating_sub(limit);

        Ok(entries[start..]
            .iter()
            .map(|entry| AuditEntryInfo {
                timestamp: DateTime::from(entry.timestamp),
                action: format!("{:?}", entry.action),
                outcome: match &entry.outcome {
                    astrid_audit::AuditOutcome::Success { details } => {
                        if let Some(d) = details {
                            format!("OK: {d}")
                        } else {
                            "OK".to_string()
                        }
                    },
                    astrid_audit::AuditOutcome::Failure { error } => {
                        format!("FAIL: {error}")
                    },
                },
            })
            .collect())
    }

    async fn save_session(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        let session = handle.session.lock().await;
        self.runtime.save_session(&session).map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INTERNAL_ERROR,
                format!("Failed to save session: {e}"),
                None::<()>,
            )
        })?;

        info!(session_id = %session_id, "Session saved via RPC");
        Ok(())
    }

    async fn list_plugins(&self) -> Result<Vec<PluginInfo>, ErrorObjectOwned> {
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

    async fn load_plugin(&self, plugin_id: String) -> Result<PluginInfo, ErrorObjectOwned> {
        let pid = PluginId::new(&plugin_id).map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("Invalid plugin id: {e}"),
                None::<()>,
            )
        })?;

        // Clear user-unloaded flag — user explicitly wants this plugin loaded.
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
        // Write lock released — other registry operations are unblocked.

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

    async fn unload_plugin(&self, plugin_id: String) -> Result<(), ErrorObjectOwned> {
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

    async fn cancel_turn(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        // Take the turn handle (if a turn is running) and abort it.
        let join_handle = handle.turn_handle.lock().await.take();
        if let Some(jh) = join_handle {
            jh.abort();
            let _ = handle.event_tx.send(DaemonEvent::TurnComplete);
            info!(session_id = %session_id, "Turn cancelled via RPC");
        }

        Ok(())
    }

    async fn subscribe_events(
        &self,
        pending: PendingSubscriptionSink,
        session_id: SessionId,
    ) -> jsonrpsee::core::SubscriptionResult {
        // Look up the session handle (brief read lock).
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        let mut event_rx = handle.event_tx.subscribe();

        let sink = pending.accept().await?;

        // Track this connection for ephemeral shutdown monitoring.
        let connections = Arc::clone(&self.active_connections);
        connections.fetch_add(1, Ordering::Relaxed);

        // Spawn a task to forward events from the broadcast channel to the subscription.
        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let msg = SubscriptionMessage::from_json(&event);
                        match msg {
                            Ok(msg) => {
                                if sink.send(msg).await.is_err() {
                                    break; // Client disconnected.
                                }
                            },
                            Err(e) => {
                                warn!("Failed to serialize event: {e}");
                            },
                        }
                    },
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "Event subscriber lagged");
                    },
                    Err(broadcast::error::RecvError::Closed) => {
                        break; // Channel closed (session ended).
                    },
                }
            }

            // Client disconnected — decrement connection count.
            connections.fetch_sub(1, Ordering::Relaxed);
        });

        Ok(())
    }
}

impl RpcImpl {
    /// Broadcast an event to all active sessions.
    ///
    /// Acquires a read lock on the session map (brief), iterates each
    /// session's `event_tx`, and sends the event. Must NOT be called while
    /// holding the plugin registry lock (deadlock risk).
    async fn broadcast_to_all_sessions(&self, event: DaemonEvent) {
        let sessions = self.sessions.read().await;
        for handle in sessions.values() {
            let _ = handle.event_tx.send(event.clone());
        }
    }

    /// Load workspace-scoped allowances from the workspace KV store.
    async fn load_workspace_allowances(&self) -> Vec<Allowance> {
        let ns = ws_ns(&self.workspace_id, "allowances");
        match self.workspace_kv.get(&ns, "all").await {
            Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Save workspace-scoped allowances to the workspace KV store.
    async fn save_workspace_allowances(&self, allowances: &[Allowance]) {
        let ns = ws_ns(&self.workspace_id, "allowances");
        if let Ok(data) = serde_json::to_vec(allowances)
            && let Err(e) = self.workspace_kv.set(&ns, "all", data).await
        {
            warn!(error = %e, "Failed to save workspace allowances");
        }
    }

    /// Load workspace escape cache from the workspace KV store.
    async fn load_workspace_escape(&self) -> Option<astrid_workspace::escape::EscapeState> {
        let ns = ws_ns(&self.workspace_id, "escape");
        match self.workspace_kv.get(&ns, "all").await {
            Ok(Some(data)) => serde_json::from_slice(&data).ok(),
            _ => None,
        }
    }

    /// Save workspace escape cache to the workspace KV store.
    async fn save_workspace_escape(&self, state: &astrid_workspace::escape::EscapeState) {
        let ns = ws_ns(&self.workspace_id, "escape");
        if let Ok(data) = serde_json::to_vec(state)
            && let Err(e) = self.workspace_kv.set(&ns, "all", data).await
        {
            warn!(error = %e, "Failed to save workspace escape state");
        }
    }

    /// Save workspace cumulative budget snapshot to the workspace KV store.
    async fn save_workspace_budget(&self) {
        let ns = ws_ns(&self.workspace_id, "budget");
        let snapshot = self.workspace_budget_tracker.snapshot();
        if let Ok(data) = serde_json::to_vec(&snapshot)
            && let Err(e) = self.workspace_kv.set(&ns, "all", data).await
        {
            warn!(error = %e, "Failed to save workspace budget");
        }
    }
}
