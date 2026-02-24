//! Daemon startup logic: configuration loading, component initialization, server binding.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use astrid_approval::budget::{WorkspaceBudgetSnapshot, WorkspaceBudgetTracker};
use astrid_audit::AuditLog;
use astrid_capabilities::CapabilityStore;
use astrid_capsule::capsule::CapsuleId;
use astrid_capsule::discovery::discover_manifests;
use astrid_capsule::registry::CapsuleRegistry;
use astrid_core::SessionId;
use astrid_core::identity::InMemoryIdentityStore;
use astrid_crypto::KeyPair;
use astrid_hooks::{HookManager, discover_hooks};
use astrid_llm::{ClaudeProvider, LlmProvider, OpenAiCompatProvider, ZaiProvider};
use astrid_mcp::McpClient;
use astrid_runtime::{AgentRuntime, SessionStore, config_bridge};
use astrid_storage::{KvStore, ScopedKvStore, SurrealKvStore};
use jsonrpsee::server::{Server, ServerHandle};
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{info, warn};
use uuid::Uuid;

use super::paths::DaemonPaths;
use super::rpc::RpcImpl;
use super::{DaemonServer, SessionHandle};
use crate::rpc::AstridRpcServer;

/// Options controlling daemon startup behaviour.
#[derive(Debug, Clone, Default)]
pub struct DaemonStartOptions {
    /// When `true`, the daemon shuts down automatically after all clients
    /// disconnect and the grace period elapses.
    pub ephemeral: bool,
    /// Override for the idle-shutdown grace period (seconds). Falls back to
    /// `gateway.idle_shutdown_secs` from the config.
    pub grace_period_secs: Option<u64>,
    /// Optional workspace root directory override. If not provided, the
    /// daemon detects the workspace from the current working directory.
    pub workspace_root: Option<PathBuf>,
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
        home_override: Option<astrid_core::dirs::AstridHome>,
    ) -> Result<(Self, ServerHandle, SocketAddr, astrid_config::Config), crate::GatewayError> {
        // Resolve and ensure directory structures.
        let home = if let Some(h) = home_override {
            h
        } else {
            astrid_core::dirs::AstridHome::resolve().map_err(|e| {
                crate::GatewayError::Runtime(format!("Failed to resolve home directory: {e}"))
            })?
        };
        home.ensure().map_err(|e| {
            crate::GatewayError::Runtime(format!(
                "Failed to create home directory {}: {e}",
                home.root().display()
            ))
        })?;

        let paths = DaemonPaths::from_dir(home.root());

        let cwd = if let Some(ref ws) = options.workspace_root {
            ws.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };
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
        let cfg = match astrid_config::Config::load_with_home(Some(&cwd), home.root()) {
            Ok(r) => r.config,
            Err(e) => {
                warn!(error = %e, "Failed to load config; falling back to defaults");
                astrid_config::Config::default()
            },
        };

        // Create LLM provider via config bridge (api key comes from config
        // precedence chain -- no redundant env var read needed here).
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
        let _mcp_for_plugins = mcp.clone();
        // Clone MCP client for watcher-driven plugin reloads.
        let _mcp_for_watcher = mcp.clone();

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

        // Discover and register capsules.
        let mut capsule_registry: CapsuleRegistry = CapsuleRegistry::new();
        let capsule_loader = astrid_capsule::loader::CapsuleLoader::new(mcp.clone());
        let plugin_dirs = vec![home.plugins_dir()];
        let discovered = discover_manifests(Some(&plugin_dirs));

        for (mut manifest, plugin_dir) in discovered {
            if let Some(component) = &mut manifest.component
                && component.entrypoint.is_relative()
            {
                let mut new_path: std::path::PathBuf = plugin_dir.clone();
                new_path.push(&component.entrypoint);
                component.entrypoint = new_path;
            }

            let capsule = match capsule_loader.create_capsule(manifest.clone(), plugin_dir.clone())
            {
                Ok(c) => c,
                Err(e) => {
                    warn!(capsule = %manifest.package.name, error = %e, "Failed to create capsule");
                    continue;
                },
            };

            if let Err(e) = capsule_registry.register(capsule) {
                warn!(capsule = %manifest.package.name, error = %e, "Failed to register capsule");
            }
        }
        let capsule_registry: Arc<RwLock<CapsuleRegistry>> =
            Arc::new(RwLock::new(capsule_registry));

        let runtime = AgentRuntime::new_arc(
            llm,
            mcp.clone(),
            audit,
            sessions,
            key,
            config,
            Some(hook_manager),
            Some(Arc::clone(&capsule_registry)),
        );

        let (shutdown_tx, _) = broadcast::channel(1);
        let session_map: Arc<RwLock<HashMap<astrid_core::SessionId, SessionHandle>>> =
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
        let ns_budget = super::rpc::workspace::ws_ns(&workspace_id, "budget");
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
        let user_unloaded_capsules: Arc<RwLock<HashSet<CapsuleId>>> =
            Arc::new(RwLock::new(HashSet::new()));

        // Central inbound message channel: all connector plugin receivers fan in here.
        let (inbound_tx, inbound_rx) = mpsc::channel::<astrid_core::InboundMessage>(256);

        // Reverse index: AstridUserId (UUID) → most recent active SessionId.
        let connector_sessions: Arc<RwLock<HashMap<Uuid, SessionId>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Pre-clone fields needed by the inbound router before rpc_impl takes ownership.
        let router_deferred_kv = Arc::clone(&deferred_kv);
        let router_capabilities_store = Arc::clone(&capabilities_store);
        let router_workspace_budget = Arc::clone(&workspace_budget_tracker);
        let router_model_name = model_name.clone();

        let rpc_impl = RpcImpl {
            runtime: Arc::clone(&runtime),
            sessions: Arc::clone(&session_map),
            plugins: Arc::clone(&capsule_registry),
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
            user_unloaded_capsules: Arc::clone(&user_unloaded_capsules),
            workspace_root: cwd.clone(),
            connector_sessions: Arc::clone(&connector_sessions),
            inbound_tx: inbound_tx.clone(),
        };

        let handle = server.start(rpc_impl.into_rpc());

        // Write PID and port files.
        let pid = std::process::id();
        std::fs::write(paths.pid_file(), pid.to_string())
            .map_err(|e| crate::GatewayError::Runtime(format!("Failed to write PID file: {e}")))?;
        std::fs::write(paths.port_file(), addr.port().to_string())
            .map_err(|e| crate::GatewayError::Runtime(format!("Failed to write port file: {e}")))?;

        info!(addr = %addr, pid = pid, "Daemon server started");

        // Identity store for resolving platform users → canonical AstridUserIds.
        // InMemoryIdentityStore is used for now; a persistent implementation can
        // be swapped in later without changing the routing logic.
        //
        // IMPORTANT: identity data is not persisted across restarts. All connector
        // user mappings are lost when the daemon exits; linked users must re-link
        // after each restart. A SurrealDB-backed implementation replaces this in
        // a follow-up phase.
        let identity_store: Arc<dyn astrid_core::identity::IdentityStore> =
            Arc::new(InMemoryIdentityStore::new());
        info!(
            "Identity store is in-memory only — connector user mappings are not \
             persisted. Linked connector users must re-link after each daemon restart."
        );

        // Apply pre-configured identity links from [[identity.links]] config.
        // Idempotent: skips entries that are already linked.
        super::config_apply::apply_identity_links(&cfg, &identity_store).await;

        // Register the native CLI connector in the plugin registry so the
        // approval fallback chain can find an interactive surface.
        //
        // Note: "native-cli" is intentionally registered without a matching
        // plugin entry — it is a synthetic connector backed directly by the
        // daemon's own DaemonFrontend, not by any loaded plugin. The
        // register_connector API warns about orphaned connectors, but this
        // one is permanent for the daemon's lifetime and cleaned up with the
        // process. Use unregister_plugin_connectors("native-cli") if removal
        // is ever needed.
        {
            let descriptor = crate::daemon_frontend::DaemonFrontend::native_connector_descriptor();
            let mut reg: tokio::sync::RwLockWriteGuard<'_, CapsuleRegistry> =
                capsule_registry.write().await;
            if let Err(e) =
                reg.register_connector(&CapsuleId::from_static("native-cli"), descriptor)
            {
                warn!(error = %e, "Failed to register native CLI connector");
            }
        }

        {
            let registry_clone: Arc<RwLock<CapsuleRegistry>> = Arc::clone(&capsule_registry);
            let kv_clone = Arc::clone(&workspace_kv);
            let workspace_root = cwd.clone();
            let inbound_tx_for_autoload = inbound_tx.clone();
            let connectors_cfg = cfg.connectors.clone();
            tokio::spawn(async move {
                let capsule_ids: Vec<CapsuleId> = {
                    let registry: tokio::sync::RwLockReadGuard<'_, CapsuleRegistry> =
                        registry_clone.read().await;
                    registry.list().into_iter().cloned().collect()
                };

                for capsule_id in capsule_ids {
                    let mut capsule = {
                        let mut registry: tokio::sync::RwLockWriteGuard<'_, CapsuleRegistry> =
                            registry_clone.write().await;
                        match registry.unregister(&capsule_id) {
                            Ok(c) => c,
                            Err(_) => continue,
                        }
                    };

                    let kv = match ScopedKvStore::new(
                        Arc::clone(&kv_clone),
                        format!("capsule:{capsule_id}"),
                    ) {
                        Ok(kv) => kv,
                        Err(e) => {
                            warn!(capsule_id = %capsule_id, error = %e, "Failed to create capsule KV scope");
                            let mut registry: tokio::sync::RwLockWriteGuard<'_, CapsuleRegistry> =
                                registry_clone.write().await;
                            let _ = registry.register(capsule);
                            continue;
                        },
                    };

                    let ctx =
                        astrid_capsule::context::CapsuleContext::new(workspace_root.clone(), kv);

                    if let Err(e) = capsule.load(&ctx).await {
                        warn!(capsule_id = %capsule_id, error = %e, "Failed to auto-load capsule");
                    } else {
                        info!(capsule_id = %capsule_id, "Auto-loaded capsule");
                        if let Some(rx) = capsule.take_inbound_rx() {
                            let tx = inbound_tx_for_autoload.clone();
                            let pid = capsule_id.to_string();
                            tokio::spawn(async move {
                                crate::server::inbound_router::forward_inbound(pid, rx, tx).await;
                            });
                        }
                    }

                    let mut registry: tokio::sync::RwLockWriteGuard<'_, CapsuleRegistry> =
                        registry_clone.write().await;
                    let _ = registry.register(capsule);
                }

                // Validate configured connectors after all capsules are auto-loaded.
                {
                    let registry: tokio::sync::RwLockReadGuard<'_, CapsuleRegistry> =
                        registry_clone.read().await;
                    for warning in super::config_apply::validate_connector_declarations(
                        &connectors_cfg,
                        &registry,
                    ) {
                        warn!("{}", warning);
                    }
                }
            });
        }

        // Floor health interval at 5s to prevent zero/tiny intervals.
        let health_interval = Duration::from_secs(cfg.gateway.health_interval_secs.max(5));

        // Resolve ephemeral grace period: CLI flag -> config default.
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
            runtime: Arc::clone(&runtime),
            sessions: Arc::clone(&session_map),
            plugins: Arc::clone(&capsule_registry),
            workspace_kv: Arc::clone(&workspace_kv),
            home: home.clone(),
            workspace_root: cwd,
            started_at: Instant::now(),
            shutdown_tx: shutdown_tx.clone(),
            paths,
            health_interval,
            ephemeral: options.ephemeral,
            ephemeral_grace_secs,
            active_connections,
            session_cleanup_interval,
            user_unloaded_capsules,
            identity_store: Arc::clone(&identity_store),
            connector_sessions: Arc::clone(&connector_sessions),
            inbound_tx,
            mcp_client: mcp.clone(),
        };

        // Spawn the inbound message router.
        let router_ctx = super::inbound_router::InboundRouterCtx {
            inbound_rx,
            identity_store,
            sessions: session_map,
            connector_sessions,
            plugins: capsule_registry,
            runtime,
            workspace_kv: Arc::clone(&workspace_kv),
            workspace_budget_tracker: router_workspace_budget,
            workspace_id,
            capabilities_store: router_capabilities_store,
            deferred_kv: router_deferred_kv,
            model_name: router_model_name,
            shutdown_rx: shutdown_tx.subscribe(),
        };
        tokio::spawn(super::inbound_router::run_inbound_router(router_ctx));

        Ok((daemon, handle, addr, cfg))
    }
}
