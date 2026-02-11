//! JSON-RPC API definition for daemon ↔ CLI communication.
//!
//! Uses jsonrpsee proc macros to define the RPC interface.
//! The daemon implements the server side; CLI implements the client side.

use std::path::PathBuf;

use astralis_core::{
    ApprovalDecision, ApprovalRequest, ElicitationRequest, ElicitationResponse, SessionId,
};
use chrono::{DateTime, Utc};
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::ErrorObjectOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------- Wire types ----------

/// Information about a session returned by create/resume operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique session identifier.
    pub id: SessionId,
    /// Workspace root this session is bound to.
    pub workspace: PathBuf,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// Number of messages in the session.
    pub message_count: usize,
    /// Number of pending deferred items that need attention.
    #[serde(default)]
    pub pending_deferred_count: usize,
}

/// Status information about the running daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    /// Whether the daemon is running.
    pub running: bool,
    /// How long the daemon has been running (seconds).
    pub uptime_secs: u64,
    /// Number of active sessions.
    pub active_sessions: usize,
    /// Daemon version.
    pub version: String,
    /// Number of configured MCP servers.
    pub mcp_servers_configured: usize,
    /// Number of running MCP servers.
    pub mcp_servers_running: usize,
    /// Whether the daemon is running in ephemeral mode (auto-shutdown when
    /// all clients disconnect).
    #[serde(default)]
    pub ephemeral: bool,
    /// Number of active `WebSocket` connections (event subscribers).
    #[serde(default)]
    pub active_connections: usize,
}

/// Status info for a single MCP server (wire type for the RPC boundary).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    /// Server name.
    pub name: String,
    /// Whether the server process is alive.
    pub alive: bool,
    /// Whether the server has completed the MCP handshake.
    pub ready: bool,
    /// Number of tools provided by this server.
    pub tool_count: usize,
    /// How many times this server has been restarted.
    pub restart_count: u32,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Budget information for a session (wire type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetInfo {
    /// Amount spent this session (USD).
    pub session_spent_usd: f64,
    /// Session budget limit (USD).
    pub session_max_usd: f64,
    /// Remaining session budget (USD).
    pub session_remaining_usd: f64,
    /// Per-action limit (USD).
    pub per_action_max_usd: f64,
    /// Warning threshold percentage.
    pub warn_at_percent: u8,
    /// Workspace cumulative spend (USD), if workspace budget is active.
    pub workspace_spent_usd: Option<f64>,
    /// Workspace budget limit (USD), if configured.
    pub workspace_max_usd: Option<f64>,
    /// Workspace remaining budget (USD), if configured.
    pub workspace_remaining_usd: Option<f64>,
}

/// Allowance information (wire type for display).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowanceInfo {
    /// Allowance ID.
    pub id: String,
    /// Human-readable pattern description.
    pub pattern: String,
    /// Whether this is session-scoped.
    pub session_only: bool,
    /// When the allowance was created.
    pub created_at: DateTime<Utc>,
    /// When the allowance expires, if ever.
    pub expires_at: Option<DateTime<Utc>>,
    /// Remaining uses, if limited.
    pub uses_remaining: Option<u32>,
}

/// Information about a single tool (wire type for the RPC boundary).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    /// Tool name.
    pub name: String,
    /// Server that provides this tool.
    pub server: String,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Audit entry summary (wire type for display).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntryInfo {
    /// Entry timestamp.
    pub timestamp: DateTime<Utc>,
    /// Action description.
    pub action: String,
    /// Outcome (success/failure).
    pub outcome: String,
}

/// Events streamed from the daemon to connected CLI clients.
///
/// These flow over a `jsonrpsee` subscription (`WebSocket` push).
/// The CLI renders them in real time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonEvent {
    /// LLM text chunk (streaming token).
    Text(String),
    /// A tool call has started.
    ToolCallStart {
        /// Call ID.
        id: String,
        /// Tool name.
        name: String,
        /// Tool arguments (may be partial during streaming).
        args: Value,
    },
    /// A tool call has produced a result.
    ToolCallResult {
        /// Call ID.
        id: String,
        /// Result content.
        result: String,
        /// Whether the tool call errored.
        is_error: bool,
    },
    /// The daemon needs the user to approve an operation.
    ApprovalNeeded {
        /// Request ID (used to correlate response).
        request_id: String,
        /// The approval request details.
        request: ApprovalRequest,
    },
    /// The daemon needs the user to provide elicitation input.
    ElicitationNeeded {
        /// Request ID (used to correlate response).
        request_id: String,
        /// The elicitation request details.
        request: ElicitationRequest,
    },
    /// Token usage update (sent after each turn).
    Usage {
        /// Estimated tokens used in the context so far.
        context_tokens: usize,
        /// Maximum context window size in tokens.
        max_context_tokens: usize,
    },
    /// The session was saved.
    SessionSaved,
    /// The current turn is complete.
    TurnComplete,
    /// An error occurred.
    Error(String),
}

// ---------- RPC API ----------

/// The Astralis daemon RPC API.
///
/// Implemented by the daemon (server side).
/// Called by the CLI (client side).
#[rpc(server, client, namespace = "astralis")]
pub trait AstralisRpc {
    /// Create a new session in a workspace.
    #[method(name = "createSession")]
    async fn create_session(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<SessionInfo, ErrorObjectOwned>;

    /// Resume an existing session.
    #[method(name = "resumeSession")]
    async fn resume_session(&self, session_id: SessionId) -> Result<SessionInfo, ErrorObjectOwned>;

    /// Send user input (events arrive via subscription).
    #[method(name = "sendInput")]
    async fn send_input(
        &self,
        session_id: SessionId,
        input: String,
    ) -> Result<(), ErrorObjectOwned>;

    /// Respond to an approval request from the daemon.
    #[method(name = "approvalResponse")]
    async fn approval_response(
        &self,
        session_id: SessionId,
        request_id: String,
        decision: ApprovalDecision,
    ) -> Result<(), ErrorObjectOwned>;

    /// Respond to an elicitation request from the daemon.
    #[method(name = "elicitationResponse")]
    async fn elicitation_response(
        &self,
        session_id: SessionId,
        request_id: String,
        response: ElicitationResponse,
    ) -> Result<(), ErrorObjectOwned>;

    /// List sessions for a workspace (or all).
    #[method(name = "listSessions")]
    async fn list_sessions(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<Vec<SessionInfo>, ErrorObjectOwned>;

    /// End a session.
    #[method(name = "endSession")]
    async fn end_session(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned>;

    /// Get daemon status.
    #[method(name = "status")]
    async fn status(&self) -> Result<DaemonStatus, ErrorObjectOwned>;

    /// List MCP servers and their status.
    #[method(name = "listServers")]
    async fn list_servers(&self) -> Result<Vec<McpServerInfo>, ErrorObjectOwned>;

    /// Start a named MCP server.
    #[method(name = "startServer")]
    async fn start_server(&self, name: String) -> Result<(), ErrorObjectOwned>;

    /// Stop a named MCP server.
    #[method(name = "stopServer")]
    async fn stop_server(&self, name: String) -> Result<(), ErrorObjectOwned>;

    /// List tools from all running MCP servers.
    #[method(name = "listTools")]
    async fn list_tools(&self) -> Result<Vec<ToolInfo>, ErrorObjectOwned>;

    /// Shutdown the daemon.
    #[method(name = "shutdown")]
    async fn shutdown(&self) -> Result<(), ErrorObjectOwned>;

    /// Get budget information for a session.
    #[method(name = "sessionBudget")]
    async fn session_budget(&self, session_id: SessionId) -> Result<BudgetInfo, ErrorObjectOwned>;

    /// Get active allowances for a session.
    #[method(name = "sessionAllowances")]
    async fn session_allowances(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<AllowanceInfo>, ErrorObjectOwned>;

    /// Get recent audit entries for a session.
    #[method(name = "sessionAudit")]
    async fn session_audit(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<AuditEntryInfo>, ErrorObjectOwned>;

    /// Explicitly save a session.
    #[method(name = "saveSession")]
    async fn save_session(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned>;

    /// Cancel the currently running turn for a session.
    #[method(name = "cancelTurn")]
    async fn cancel_turn(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned>;

    /// Subscribe to session events (real-time streaming).
    #[subscription(name = "subscribeEvents" => "event", unsubscribe = "unsubscribeEvents", item = DaemonEvent)]
    async fn subscribe_events(&self, session_id: SessionId) -> jsonrpsee::core::SubscriptionResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_info_serde_round_trip() {
        let tool = ToolInfo {
            name: "read_file".to_string(),
            server: "filesystem".to_string(),
            description: Some("Read a file".to_string()),
        };

        let json = serde_json::to_string(&tool).unwrap();
        let deserialized: ToolInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, "read_file");
        assert_eq!(deserialized.server, "filesystem");
        assert_eq!(deserialized.description, Some("Read a file".to_string()));
    }

    #[test]
    fn tool_info_without_description() {
        let json = r#"{"name":"exec","server":"shell","description":null}"#;
        let tool: ToolInfo = serde_json::from_str(json).unwrap();

        assert_eq!(tool.name, "exec");
        assert!(tool.description.is_none());
    }
}

/// Error codes for the RPC API.
pub mod error_codes {
    /// Session not found.
    pub const SESSION_NOT_FOUND: i32 = -32001;
    /// Session already exists.
    pub const SESSION_ALREADY_EXISTS: i32 = -32002;
    /// Daemon is shutting down.
    pub const DAEMON_SHUTTING_DOWN: i32 = -32003;
    /// Internal daemon error.
    pub const INTERNAL_ERROR: i32 = -32004;
    /// Invalid request (bad parameters, etc.).
    pub const INVALID_REQUEST: i32 = -32005;
}
