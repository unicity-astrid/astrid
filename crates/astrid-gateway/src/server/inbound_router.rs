//! Inbound message broker: fans in all connector receivers and routes messages
//! to agent sessions by identity.
//!
//! # Design
//!
//! Each loaded plugin that declares `PluginCapability::Connector` holds an
//! `mpsc::Receiver<InboundMessage>` that it produces inbound messages on. The
//! gateway calls [`forward_inbound`] per plugin to fan all of those receivers
//! into a single central channel. The [`run_inbound_router`] task drains that
//! central channel and resolves each message to an [`AgentSession`].
//!
//! # Security
//!
//! Messages from unknown users are dropped (fail-secure). The full pairing
//! flow (generate link code + outbound response) is a follow-up once
//! `OutboundAdapter` is on the `Plugin` trait.
//!
//! # Locking
//!
//! Sessions and the `connector_sessions` reverse index are **never held across
//! async boundaries**. Locks are taken briefly for lookup/insert, then released
//! before any `await`-ed work (LLM calls, identity store queries, etc.).

use std::collections::HashMap;
use std::sync::Arc;

use astrid_approval::budget::WorkspaceBudgetTracker;
use astrid_capabilities::CapabilityStore;
use astrid_core::InboundMessage;
use astrid_core::SessionId;
use astrid_core::identity::IdentityStore;
use astrid_llm::LlmProvider;
use astrid_plugins::PluginRegistry;
use astrid_runtime::AgentRuntime;
use astrid_storage::{KvStore, ScopedKvStore};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use tracing::{info, warn};
use uuid::Uuid;

use super::SessionHandle;
use super::rpc::workspace::ws_ns;
use crate::daemon_frontend::DaemonFrontend;
use crate::rpc::DaemonEvent;

// ---------------------------------------------------------------------------
// Fan-in forwarder (one per plugin)
// ---------------------------------------------------------------------------

/// Forward messages from a plugin's inbound receiver to the central channel.
///
/// Runs until the plugin drops its sender (plugin unloaded) or the central
/// channel closes (daemon shutting down). One task is spawned per plugin.
pub(super) async fn forward_inbound(
    plugin_id: String,
    mut rx: mpsc::Receiver<InboundMessage>,
    tx: mpsc::Sender<InboundMessage>,
) {
    while let Some(msg) = rx.recv().await {
        if tx.send(msg).await.is_err() {
            warn!(plugin = %plugin_id, "Central inbound channel closed — stopping forwarder");
            break;
        }
    }
    info!(plugin = %plugin_id, "Plugin inbound forwarder ended");
}

// ---------------------------------------------------------------------------
// Router context
// ---------------------------------------------------------------------------

/// All state needed by the inbound router background task.
pub(super) struct InboundRouterCtx {
    pub inbound_rx: mpsc::Receiver<InboundMessage>,
    pub identity_store: Arc<dyn IdentityStore>,
    pub sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    /// Maps canonical `AstridUserId` (UUID) → most recent active `SessionId`.
    pub connector_sessions: Arc<RwLock<HashMap<Uuid, SessionId>>>,
    /// Stored for future use by the approval fallback chain (finding connectors
    /// with approval capability when a connector session needs user approval).
    #[allow(dead_code)]
    pub plugin_registry: Arc<RwLock<PluginRegistry>>,
    pub runtime: Arc<AgentRuntime<Box<dyn LlmProvider>>>,
    pub workspace_kv: Arc<dyn KvStore>,
    pub workspace_budget_tracker: Arc<WorkspaceBudgetTracker>,
    pub workspace_id: Uuid,
    pub capabilities_store: Arc<CapabilityStore>,
    pub deferred_kv: Arc<dyn KvStore>,
    pub model_name: String,
    pub shutdown_rx: broadcast::Receiver<()>,
}

// ---------------------------------------------------------------------------
// Main router task
// ---------------------------------------------------------------------------

/// Run the inbound message router.
///
/// Spawned once during daemon startup. Loops until the inbound channel closes
/// or a shutdown signal is received.
///
/// # Shutdown behaviour
///
/// Uses a `biased` select that checks the shutdown signal before the inbound
/// channel. On shutdown, any messages already buffered in the central channel
/// (up to 256) are dropped without processing. This is acceptable for the
/// current phase because there is no outbound reply path yet; once
/// `OutboundAdapter` lands, consider a two-phase drain (flush remaining
/// messages before breaking) to preserve at-least-once delivery.
pub(super) async fn run_inbound_router(mut ctx: InboundRouterCtx) {
    loop {
        tokio::select! {
            biased;
            result = ctx.shutdown_rx.recv() => {
                // Treat all outcomes as shutdown: Ok(()) = signal received;
                // Lagged = missed the signal (daemon already shutting down);
                // Closed = sender dropped (daemon exiting). All mean: stop.
                let _ = result;
                info!("Inbound router received shutdown signal");
                break;
            }
            msg = ctx.inbound_rx.recv() => {
                if let Some(msg) = msg {
                    handle_inbound(&ctx, msg).await;
                } else {
                    info!("Inbound channel closed — router exiting");
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Message handling
// ---------------------------------------------------------------------------

async fn handle_inbound(ctx: &InboundRouterCtx, msg: InboundMessage) {
    let user = ctx
        .identity_store
        .resolve(&msg.platform, &msg.platform_user_id)
        .await;

    match user {
        Some(astrid_user) => {
            let session_id = find_or_create_session(ctx, astrid_user.id).await;
            if let Some(session_id) = session_id {
                run_connector_turn(ctx, session_id, msg.content).await;
            }
        },
        None => {
            // Fail-secure: unknown user → log and drop.
            // Full pairing flow (generate_link_code + OutboundAdapter) is a follow-up
            // once `OutboundAdapter` is exposed on the `Plugin` trait.
            warn!(
                platform = ?msg.platform,
                user_id = %msg.platform_user_id,
                "Inbound message from unknown user — dropping (pairing flow pending)"
            );
        },
    }
}

// ---------------------------------------------------------------------------
// Session management
// ---------------------------------------------------------------------------

/// Find the existing session for `user_id`, or create a new one.
///
/// Stale entries in `connector_sessions` (where the referenced session has
/// been cleaned up by the session cleanup loop) are treated identically to
/// missing entries: a new session is created and the map is updated.
///
/// # Concurrency
///
/// This function is only called from the single inbound router task
/// (`run_inbound_router`). Because the router processes messages
/// sequentially (one `handle_inbound` completes before the next begins),
/// the read-check-then-write pattern below is free of TOCTOU races. Do not
/// call this from multiple concurrent tasks without adding a per-user lock.
///
/// # Workspace
///
/// Connector sessions are not scoped to a workspace (Phase 5 limitation).
/// They operate at the top level. Per-user workspace assignment is a
/// follow-up once the identity/pairing flow is complete.
async fn find_or_create_session(ctx: &InboundRouterCtx, user_id: Uuid) -> Option<SessionId> {
    // Brief read — look for an existing, live session.
    {
        let cs = ctx.connector_sessions.read().await;
        if let Some(sid) = cs.get(&user_id) {
            let live = ctx.sessions.read().await.contains_key(sid);
            if live {
                return Some(sid.clone());
            }
            // Session was cleaned up — fall through to creation.
        }
    }

    // Build a new session with the same setup as `create_session_impl`.
    let mut session = ctx.runtime.create_session(None);
    session.model = Some(ctx.model_name.clone());
    let session = session.with_capability_store(Arc::clone(&ctx.capabilities_store));

    let scoped = match ScopedKvStore::new(
        Arc::clone(&ctx.deferred_kv),
        format!("deferred:{}", session.id.0),
    ) {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %user_id, "Failed to create deferred KV scope for connector session");
            return None;
        },
    };
    let session = match session.with_persistent_deferred_queue(scoped).await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %user_id, "Failed to init deferred queue for connector session");
            return None;
        },
    };
    let mut session = session.with_workspace_budget(Arc::clone(&ctx.workspace_budget_tracker));

    // Load workspace-scoped allowances.
    let ns_allowances = ws_ns(&ctx.workspace_id, "allowances");
    if let Ok(Some(data)) = ctx.workspace_kv.get(&ns_allowances, "all").await
        && let Ok(allowances) = serde_json::from_slice(&data)
    {
        session.import_workspace_allowances(allowances);
    }

    // Load workspace escape cache.
    let ns_escape = ws_ns(&ctx.workspace_id, "escape");
    if let Ok(Some(data)) = ctx.workspace_kv.get(&ns_escape, "all").await
        && let Ok(state) = serde_json::from_slice(&data)
    {
        session.escape_handler.restore_state(state);
    }

    let session_id = session.id.clone();
    let created_at = session.created_at;
    let (event_tx, _) = broadcast::channel(256);
    let frontend = Arc::new(DaemonFrontend::new(event_tx.clone()));

    let handle = SessionHandle {
        session: Arc::new(Mutex::new(session)),
        frontend,
        event_tx,
        workspace: None,
        created_at,
        turn_handle: Arc::new(Mutex::new(None)),
        user_id: Some(user_id),
    };

    // Insert into both maps (brief write locks, not held concurrently).
    {
        let mut sessions = ctx.sessions.write().await;
        sessions.insert(session_id.clone(), handle);
    }
    {
        let mut cs = ctx.connector_sessions.write().await;
        cs.insert(user_id, session_id.clone());
    }

    info!(%session_id, %user_id, "Created connector session");
    Some(session_id)
}

// ---------------------------------------------------------------------------
// Turn execution
// ---------------------------------------------------------------------------

/// Run a connector-originated agent turn.
///
/// Mirrors `send_input_impl` exactly:
/// - Spawns the turn in a background `tokio::task`.
/// - Stores the `JoinHandle` in `handle.turn_handle` so `cancel_turn` works.
/// - Auto-saves and persists workspace state after each turn.
///
/// # Concurrent messages
///
/// If a second message arrives from the same user before the first turn
/// finishes, a second task is spawned. The per-session `AgentSession` Mutex
/// serialises execution so the turns run sequentially in practice. The
/// `turn_handle` will hold the most-recently-spawned handle; `cancel_turn`
/// cancels whichever task is referenced at the time it is called.
async fn run_connector_turn(ctx: &InboundRouterCtx, session_id: SessionId, input: String) {
    // Brief read lock to clone the handle.
    let handle = {
        let sessions = ctx.sessions.read().await;
        if let Some(h) = sessions.get(&session_id).cloned() {
            h
        } else {
            warn!(%session_id, "Session vanished before connector turn could start");
            return;
        }
    };

    let runtime = Arc::clone(&ctx.runtime);
    let event_tx = handle.event_tx.clone();
    let frontend = Arc::clone(&handle.frontend);
    let session_mutex = Arc::clone(&handle.session);
    let workspace_kv = Arc::clone(&ctx.workspace_kv);
    let ws_budget_tracker = Arc::clone(&ctx.workspace_budget_tracker);
    let ws_id = ctx.workspace_id;
    let turn_handle = Arc::clone(&handle.turn_handle);

    let join_handle = tokio::spawn(async move {
        let mut session = session_mutex.lock().await;

        let result = runtime
            .run_turn_streaming(&mut session, &input, Arc::clone(&frontend))
            .await;

        // Auto-save after every turn for crash recovery.
        if let Err(e) = runtime.save_session(&session) {
            warn!(error = %e, "Failed to auto-save connector session after turn");
        } else {
            let _ = event_tx.send(DaemonEvent::SessionSaved);
        }

        // Persist workspace-scoped allowances.
        let ws_allowances = session.export_workspace_allowances();
        if !ws_allowances.is_empty()
            && let Ok(data) = serde_json::to_vec(&ws_allowances)
            && let Err(e) = workspace_kv
                .set(&ws_ns(&ws_id, "allowances"), "all", data)
                .await
        {
            warn!(error = %e, "Failed to save workspace allowances after connector turn");
        }

        // Persist workspace budget snapshot.
        {
            let snapshot = ws_budget_tracker.snapshot();
            if let Ok(data) = serde_json::to_vec(&snapshot)
                && let Err(e) = workspace_kv
                    .set(&ws_ns(&ws_id, "budget"), "all", data)
                    .await
            {
                warn!(error = %e, "Failed to save workspace budget after connector turn");
            }
        }

        // Persist workspace escape cache.
        {
            let escape_state = session.escape_handler.export_state();
            if !escape_state.remembered_paths.is_empty()
                && let Ok(data) = serde_json::to_vec(&escape_state)
                && let Err(e) = workspace_kv
                    .set(&ws_ns(&ws_id, "escape"), "all", data)
                    .await
            {
                warn!(error = %e, "Failed to save workspace escape state after connector turn");
            }
        }

        // Context usage update.
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

    // Store the join handle so cancel_turn can abort this task.
    // Both this code and the spawned task's cleanup share the same Mutex, so
    // only one can run at a time: if the task finishes first it clears the
    // handle and we see is_finished() == true here (leaving None); if we
    // store first the task will clear it when it completes.
    {
        let mut guard = handle.turn_handle.lock().await;
        if !join_handle.is_finished() {
            *guard = Some(join_handle);
        }
        // If already finished: the task cleared turn_handle itself; dropping
        // the finished JoinHandle here is a no-op.
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use astrid_core::FrontendType;
    use astrid_core::connector::ConnectorId;
    use astrid_core::identity::{IdentityStore, InMemoryIdentityStore};
    use tokio::sync::RwLock;
    use uuid::Uuid;

    #[tokio::test]
    async fn unknown_user_resolve_returns_none() {
        let store = InMemoryIdentityStore::new();
        let result = store
            .resolve(&FrontendType::Telegram, "unknown_telegram_42")
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn known_user_resolve_returns_identity() {
        let store = InMemoryIdentityStore::new();
        let user = store
            .create_identity(FrontendType::Telegram, "tg_user_1")
            .await
            .expect("create identity");
        let resolved = store.resolve(&FrontendType::Telegram, "tg_user_1").await;
        assert_eq!(resolved.map(|u| u.id), Some(user.id));
    }

    /// Verifies the data-model invariant: a connector_sessions entry that
    /// references a session not present in the sessions map is "stale".
    /// The router's `find_or_create_session` detects this and creates a new
    /// session rather than routing to a dead one.
    #[tokio::test]
    async fn connector_sessions_stale_entry_detected() {
        // Simulate: connector_sessions has user→session but sessions map is empty.
        let connector_sessions: Arc<RwLock<HashMap<Uuid, astrid_core::SessionId>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let sessions: Arc<RwLock<HashMap<astrid_core::SessionId, super::SessionHandle>>> =
            Arc::new(RwLock::new(HashMap::new()));

        let user_id = Uuid::new_v4();
        let stale_session_id = astrid_core::SessionId(Uuid::new_v4());

        // Insert a stale entry.
        connector_sessions
            .write()
            .await
            .insert(user_id, stale_session_id.clone());

        // Verify: sessions doesn't contain it (stale).
        let cs = connector_sessions.read().await;
        let sid = cs.get(&user_id).unwrap();
        let live = sessions.read().await.contains_key(sid);
        assert!(!live, "stale session should not be in sessions map");
    }

    /// `forward_inbound` relays messages from a plugin's receiver to the
    /// central inbound channel unchanged.
    #[tokio::test]
    async fn forward_inbound_relays_messages() {
        use astrid_core::InboundMessage;
        use tokio::sync::mpsc;

        let (plugin_tx, plugin_rx) = mpsc::channel::<InboundMessage>(8);
        let (central_tx, mut central_rx) = mpsc::channel(8);

        tokio::spawn(super::forward_inbound(
            "test-plugin".to_string(),
            plugin_rx,
            central_tx,
        ));

        let msg = InboundMessage::builder(
            ConnectorId::new(),
            FrontendType::Telegram,
            "user-42",
            "hello from connector",
        )
        .build();

        plugin_tx.send(msg).await.unwrap();

        let received = central_rx.recv().await.expect("message forwarded");
        assert_eq!(received.content, "hello from connector");
        assert_eq!(received.platform_user_id, "user-42");
    }

    /// `forward_inbound` exits cleanly when the plugin sender is dropped
    /// (plugin unloaded), without blocking or panicking.
    #[tokio::test]
    async fn forward_inbound_exits_when_plugin_sender_drops() {
        use astrid_core::InboundMessage;
        use tokio::sync::mpsc;

        let (plugin_tx, plugin_rx) = mpsc::channel::<InboundMessage>(8);
        // `_central_rx` is dropped at end of scope, closing the central channel.
        // The forwarder exits via whichever path fires first: plugin sender
        // drop (rx.recv() → None) or central channel close (tx.send → Err).
        // Both are correct self-termination paths; the test verifies no panic.
        let (central_tx, _central_rx) = mpsc::channel(8);

        let handle = tokio::spawn(super::forward_inbound(
            "test-plugin".to_string(),
            plugin_rx,
            central_tx,
        ));

        // Drop the plugin sender — the forwarder task should exit cleanly.
        drop(plugin_tx);
        handle
            .await
            .expect("forwarder task should complete without panic");
    }
}
