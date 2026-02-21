//! Session RPC method implementations.

use std::path::PathBuf;
use std::sync::Arc;

use astrid_core::SessionId;
use astrid_storage::ScopedKvStore;
use jsonrpsee::types::ErrorObjectOwned;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::RpcImpl;
use super::workspace::ws_ns;
use crate::daemon_frontend::DaemonFrontend;
use crate::rpc::{DaemonEvent, SessionInfo, error_codes};
use crate::server::SessionHandle;

impl RpcImpl {
    pub(super) async fn create_session_impl(
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
        let (event_tx, _) = tokio::sync::broadcast::channel(256);
        let frontend = Arc::new(DaemonFrontend::new(event_tx.clone()));

        let handle = SessionHandle {
            session: Arc::new(Mutex::new(session)),
            frontend,
            event_tx,
            workspace: workspace_path.clone(),
            created_at,
            turn_handle: Arc::new(Mutex::new(None)),
            user_id: None,
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

    #[allow(clippy::too_many_lines)]
    pub(super) async fn resume_session_impl(
        &self,
        session_id: SessionId,
    ) -> Result<SessionInfo, ErrorObjectOwned> {
        // Check if already live (brief read lock).
        {
            let sessions = self.sessions.read().await;
            if let Some(handle) = sessions.get(&session_id) {
                // Connector sessions are managed by the inbound router; RPC
                // callers must not re-enter them (no outbound event path).
                if handle.user_id.is_some() {
                    return Err(ErrorObjectOwned::owned(
                        error_codes::INVALID_REQUEST,
                        "session is managed by the inbound router and cannot be resumed via RPC",
                        None::<()>,
                    ));
                }
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

        // Guard against resuming a connector session that was saved to disk.
        // connector_sessions maps AstridUserId → SessionId; a reverse lookup
        // detects sessions whose ID is still indexed (user hasn't sent a new
        // message since the session was evicted from the live map).
        //
        // TOCTOU note: between this check and the disk load below, the inbound
        // router could concurrently create a live session for the same user. A
        // write-time conflict check (or a per-session creation lock) would close
        // this window but is out of scope for now — the race is narrow and
        // requires the connector user to send a message at exactly this moment.
        {
            let cs = self.connector_sessions.read().await;
            if cs.values().any(|sid| sid == &session_id) {
                return Err(ErrorObjectOwned::owned(
                    error_codes::INVALID_REQUEST,
                    "session is managed by the inbound router and cannot be resumed via RPC",
                    None::<()>,
                ));
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

        // Guard against resuming a connector session from disk.
        // Connector sessions are tagged at creation in `inbound_router::find_or_create_session`.
        if session.metadata.custom.contains_key("connector_user_id") {
            return Err(ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                "session is managed by the inbound router and cannot be resumed via RPC",
                None::<()>,
            ));
        }

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

        let (event_tx, _) = tokio::sync::broadcast::channel(256);
        let frontend = Arc::new(DaemonFrontend::new(event_tx.clone()));

        let handle = SessionHandle {
            session: Arc::new(Mutex::new(session)),
            frontend,
            event_tx,
            workspace: workspace.clone(),
            created_at,
            turn_handle: Arc::new(Mutex::new(None)),
            user_id: None,
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

    pub(super) async fn send_input_impl(
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

        // Connector sessions are managed exclusively by the inbound router.
        // Sending input via RPC would bypass identity resolution and produce
        // turn output with no outbound path back to the connector user.
        if handle.user_id.is_some() {
            return Err(ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                "session is managed by the inbound router and cannot be targeted via RPC",
                None::<()>,
            ));
        }

        let runtime = Arc::clone(&self.runtime);
        let event_tx = handle.event_tx.clone();
        let frontend = Arc::clone(&handle.frontend);
        let session_mutex = Arc::clone(&handle.session);
        let workspace_kv = Arc::clone(&self.workspace_kv);
        let ws_budget_tracker = Arc::clone(&self.workspace_budget_tracker);
        let ws_id = self.workspace_id;
        let turn_handle = Arc::clone(&handle.turn_handle);

        // Run the agent turn in a background task.
        // Only the per-session mutex is held -- other sessions, approval_response,
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
        // Acquire the lock before checking is_finished() to close the window
        // where the task could finish and clear turn_handle between spawn() and
        // this assignment. If the task already finished, leave the handle as None.
        {
            let mut guard = handle.turn_handle.lock().await;
            if !join_handle.is_finished() {
                *guard = Some(join_handle);
            }
            // If already finished: the task cleared turn_handle itself; dropping
            // the finished JoinHandle here is a no-op.
        }

        Ok(())
    }

    pub(super) async fn list_sessions_impl(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<Vec<SessionInfo>, ErrorObjectOwned> {
        // Collect handles under a brief read lock; do not hold the read lock
        // across per-session Mutex awaits — that would stall the entire session
        // map behind any slow LLM turn, violating the locking discipline.
        //
        // Connector-originated sessions (user_id.is_some()) are excluded: they
        // have no outbound event path, so a caller who receives their ID and
        // tries to send_input would run an LLM turn whose output goes nowhere.
        let handles: Vec<(SessionId, SessionHandle)> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter(|(_, handle)| {
                    handle.user_id.is_none()
                        && workspace_path
                            .as_ref()
                            .is_none_or(|ws| handle.workspace.as_ref() == Some(ws))
                })
                .map(|(id, handle)| (id.clone(), handle.clone()))
                .collect()
        };

        let mut result = Vec::new();
        for (id, handle) in handles {
            let session = handle.session.lock().await;
            let pending_deferred_count = session.approval_manager.get_pending_resolutions().len();
            result.push(SessionInfo {
                id,
                workspace: handle.workspace.clone(),
                created_at: handle.created_at,
                message_count: session.messages.len(),
                pending_deferred_count,
            });
        }

        Ok(result)
    }

    pub(super) async fn end_session_impl(
        &self,
        session_id: SessionId,
    ) -> Result<(), ErrorObjectOwned> {
        // Remove the session from the map if it belongs to a CLI user (brief write lock).
        let handle = {
            let mut sessions = self.sessions.write().await;

            // First get a reference to check user_id before removing.
            let is_connector = sessions
                .get(&session_id)
                .is_some_and(|h| h.user_id.is_some());
            if is_connector {
                return Err(ErrorObjectOwned::owned(
                    error_codes::INVALID_REQUEST,
                    "session is managed by the inbound router and cannot be ended via RPC",
                    None::<()>,
                ));
            }

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

        // Evict plugin KV stores for this session (prevents unbounded growth).
        self.runtime.cleanup_plugin_kv_stores(&session_id);

        // Save session before ending.
        if let Err(e) = self.runtime.save_session(&session) {
            warn!(session_id = %session_id, error = %e, "Failed to save session on end");
        }

        info!(session_id = %session_id, "Session ended via RPC");
        Ok(())
    }

    pub(super) async fn save_session_impl(
        &self,
        session_id: SessionId,
    ) -> Result<(), ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            let h = sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?;

            if h.user_id.is_some() {
                return Err(ErrorObjectOwned::owned(
                    error_codes::INVALID_REQUEST,
                    "session is managed by the inbound router and cannot be saved via RPC",
                    None::<()>,
                ));
            }
            h
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
}
