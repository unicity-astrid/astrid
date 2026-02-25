//! Daemon client for the Telegram bot.
//!
//! This is a thin wrapper around [`astrid_frontend_common::DaemonClient`]
//! that maps errors to [`TelegramBotError`].

use std::path::PathBuf;

use astrid_core::{ApprovalDecision, ElicitationResponse, SessionId};
use astrid_gateway::rpc::{BudgetInfo, DaemonEvent, DaemonStatus, SessionInfo};

use crate::error::TelegramBotError;

/// A client that connects to the Astrid daemon via `WebSocket`.
///
/// Wraps the shared [`astrid_frontend_common::DaemonClient`] and maps all
/// errors to [`TelegramBotError`] for backward compatibility.
pub struct DaemonClient {
    inner: astrid_frontend_common::DaemonClient,
}

impl DaemonClient {
    /// Connect to the daemon at the given URL.
    pub async fn connect_url(url: &str) -> Result<Self, TelegramBotError> {
        let inner = astrid_frontend_common::DaemonClient::connect_url(url)
            .await
            .map_err(map_err)?;
        Ok(Self { inner })
    }

    /// Connect to the daemon, auto-discovering the port from
    /// `~/.astrid/daemon.port`.
    pub async fn connect_discover() -> Result<Self, TelegramBotError> {
        let inner = astrid_frontend_common::DaemonClient::connect_discover()
            .await
            .map_err(map_err)?;
        Ok(Self { inner })
    }

    /// Connect using an explicit URL or fall back to auto-discovery.
    pub async fn connect(daemon_url: Option<&str>) -> Result<Self, TelegramBotError> {
        let inner = astrid_frontend_common::DaemonClient::connect(daemon_url)
            .await
            .map_err(map_err)?;
        Ok(Self { inner })
    }

    /// Create a new session.
    pub async fn create_session(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<SessionInfo, TelegramBotError> {
        self.inner
            .create_session(workspace_path)
            .await
            .map_err(map_err)
    }

    /// End a session.
    pub async fn end_session(&self, session_id: &SessionId) -> Result<(), TelegramBotError> {
        self.inner.end_session(session_id).await.map_err(map_err)
    }

    /// Send user input to a session.
    pub async fn send_input(
        &self,
        session_id: &SessionId,
        input: &str,
    ) -> Result<(), TelegramBotError> {
        self.inner
            .send_input(session_id, input)
            .await
            .map_err(map_err)
    }

    /// Subscribe to session events.
    pub async fn subscribe_events(
        &self,
        session_id: &SessionId,
    ) -> Result<jsonrpsee::core::client::Subscription<DaemonEvent>, TelegramBotError> {
        self.inner
            .subscribe_events(session_id)
            .await
            .map_err(map_err)
    }

    /// Respond to an approval request.
    pub async fn send_approval(
        &self,
        session_id: &SessionId,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), TelegramBotError> {
        self.inner
            .send_approval(session_id, request_id, decision)
            .await
            .map_err(map_err)
    }

    /// Respond to an elicitation request.
    pub async fn send_elicitation(
        &self,
        session_id: &SessionId,
        request_id: &str,
        response: ElicitationResponse,
    ) -> Result<(), TelegramBotError> {
        self.inner
            .send_elicitation(session_id, request_id, response)
            .await
            .map_err(map_err)
    }

    /// Cancel the current turn.
    pub async fn cancel_turn(&self, session_id: &SessionId) -> Result<(), TelegramBotError> {
        self.inner.cancel_turn(session_id).await.map_err(map_err)
    }

    /// Get daemon status.
    pub async fn status(&self) -> Result<DaemonStatus, TelegramBotError> {
        self.inner.status().await.map_err(map_err)
    }

    /// Get budget info for a session.
    pub async fn session_budget(
        &self,
        session_id: &SessionId,
    ) -> Result<BudgetInfo, TelegramBotError> {
        self.inner.session_budget(session_id).await.map_err(map_err)
    }
}

/// Map a [`FrontendCommonError`] to a [`TelegramBotError`].
fn map_err(e: astrid_frontend_common::FrontendCommonError) -> TelegramBotError {
    use astrid_frontend_common::FrontendCommonError;
    match e {
        FrontendCommonError::DaemonConnection(msg) => TelegramBotError::DaemonConnection(msg),
        FrontendCommonError::DaemonRpc(msg) => TelegramBotError::DaemonRpc(msg),
        FrontendCommonError::Config(msg) => TelegramBotError::Config(msg),
    }
}
