//! Platform-agnostic daemon client for frontend bots.
//!
//! Wraps a `jsonrpsee` `WebSocket` client and exposes typed methods for every
//! RPC call. Unlike the CLI client, this does **not** auto-start the daemon —
//! the daemon must already be running.

use std::path::PathBuf;
use std::time::Duration;

use astrid_core::{ApprovalDecision, ElicitationResponse, SessionId};
use astrid_gateway::rpc::{AstridRpcClient, BudgetInfo, DaemonEvent, DaemonStatus, SessionInfo};
use astrid_gateway::server::DaemonPaths;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};

use crate::error::FrontendCommonError;

/// A client that connects to the Astrid daemon via `WebSocket`.
///
/// Shared by all frontend crates (Telegram, Discord, etc.). The daemon
/// must already be running — this client does **not** auto-start it.
pub struct DaemonClient {
    client: WsClient,
}

impl DaemonClient {
    /// Connect to the daemon at the given URL.
    pub async fn connect_url(url: &str) -> Result<Self, FrontendCommonError> {
        let client = WsClientBuilder::default()
            .connection_timeout(Duration::from_secs(10))
            .build(url)
            .await
            .map_err(|e| {
                FrontendCommonError::DaemonConnection(format!(
                    "failed to connect to daemon at {url}: {e}"
                ))
            })?;

        Ok(Self { client })
    }

    /// Connect to the daemon, auto-discovering the port from
    /// `~/.astrid/daemon.port`.
    pub async fn connect_discover() -> Result<Self, FrontendCommonError> {
        let paths = DaemonPaths::default_dir()
            .map_err(|e| FrontendCommonError::DaemonConnection(e.to_string()))?;

        let port = astrid_gateway::DaemonServer::read_port(&paths).ok_or_else(|| {
            FrontendCommonError::DaemonConnection(
                "daemon port file not found — is astridd running?".to_string(),
            )
        })?;

        let url = format!("ws://127.0.0.1:{port}");
        Self::connect_url(&url).await
    }

    /// Connect using an explicit URL or fall back to auto-discovery.
    pub async fn connect(daemon_url: Option<&str>) -> Result<Self, FrontendCommonError> {
        match daemon_url {
            Some(url) => Self::connect_url(url).await,
            None => Self::connect_discover().await,
        }
    }

    /// Create a new session.
    pub async fn create_session(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<SessionInfo, FrontendCommonError> {
        self.client
            .create_session(workspace_path)
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// End a session.
    pub async fn end_session(&self, session_id: &SessionId) -> Result<(), FrontendCommonError> {
        self.client
            .end_session(session_id.clone())
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// Send user input to a session.
    pub async fn send_input(
        &self,
        session_id: &SessionId,
        input: &str,
    ) -> Result<(), FrontendCommonError> {
        self.client
            .send_input(session_id.clone(), input.to_string())
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// Subscribe to session events.
    pub async fn subscribe_events(
        &self,
        session_id: &SessionId,
    ) -> Result<jsonrpsee::core::client::Subscription<DaemonEvent>, FrontendCommonError> {
        self.client
            .subscribe_events(session_id.clone())
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// Respond to an approval request.
    pub async fn send_approval(
        &self,
        session_id: &SessionId,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), FrontendCommonError> {
        self.client
            .approval_response(session_id.clone(), request_id.to_string(), decision)
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// Respond to an elicitation request.
    pub async fn send_elicitation(
        &self,
        session_id: &SessionId,
        request_id: &str,
        response: ElicitationResponse,
    ) -> Result<(), FrontendCommonError> {
        self.client
            .elicitation_response(session_id.clone(), request_id.to_string(), response)
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// Cancel the current turn.
    pub async fn cancel_turn(&self, session_id: &SessionId) -> Result<(), FrontendCommonError> {
        self.client
            .cancel_turn(session_id.clone())
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// Get daemon status.
    pub async fn status(&self) -> Result<DaemonStatus, FrontendCommonError> {
        self.client
            .status()
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }

    /// Get budget info for a session.
    pub async fn session_budget(
        &self,
        session_id: &SessionId,
    ) -> Result<BudgetInfo, FrontendCommonError> {
        self.client
            .session_budget(session_id.clone())
            .await
            .map_err(|e| FrontendCommonError::DaemonRpc(e.to_string()))
    }
}
