//! RPC implementation for the daemon server.
//!
//! The `RpcImpl` struct holds all shared state and implements `AstridRpcServer`
//! by delegating to `*_impl` methods in focused submodules.

mod approval;
mod budget;
mod events;
mod mcp_servers;
mod plugins;
mod session;
pub(super) mod workspace;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use astrid_approval::budget::WorkspaceBudgetTracker;
use astrid_capabilities::CapabilityStore;
use astrid_core::{ApprovalDecision, ElicitationResponse, SessionId};
use astrid_llm::LlmProvider;
use astrid_plugins::{PluginId, PluginRegistry};
use astrid_runtime::AgentRuntime;
use astrid_storage::KvStore;
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::types::ErrorObjectOwned;
use tokio::sync::{RwLock, broadcast};

use super::SessionHandle;
use crate::rpc::{
    AllowanceInfo, AstridRpcServer, AuditEntryInfo, BudgetInfo, DaemonStatus, McpServerInfo,
    PluginInfo, SessionInfo, ToolInfo,
};

/// The jsonrpsee RPC method handler.
///
/// Uses per-session locking to avoid the deadlock where `send_input`
/// (running an LLM turn) blocks `approval_response` (delivering the
/// approval that the turn is waiting for).
pub(in crate::server) struct RpcImpl {
    /// The agent runtime (immutable, never locked).
    pub(in crate::server) runtime: Arc<AgentRuntime<Box<dyn LlmProvider>>>,
    /// Session map (brief locks for insert/remove/lookup only).
    pub(in crate::server) sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    /// Plugin registry (shared, behind `RwLock`).
    pub(in crate::server) plugin_registry: Arc<RwLock<PluginRegistry>>,
    /// Shared KV store for deferred resolution persistence.
    pub(in crate::server) deferred_kv: Arc<dyn KvStore>,
    /// Shared persistent capability store (tokens survive restarts).
    pub(in crate::server) capabilities_store: Arc<CapabilityStore>,
    /// Shared workspace state KV store (allowances, budget, escape).
    pub(in crate::server) workspace_kv: Arc<dyn KvStore>,
    /// Workspace cumulative budget tracker (shared across sessions).
    pub(in crate::server) workspace_budget_tracker: Arc<WorkspaceBudgetTracker>,
    /// When the daemon started.
    pub(in crate::server) started_at: Instant,
    /// Shutdown signal.
    pub(in crate::server) shutdown_tx: broadcast::Sender<()>,
    /// Workspace UUID for namespacing KV keys.
    pub(in crate::server) workspace_id: uuid::Uuid,
    /// Model name from config (set on sessions).
    pub(in crate::server) model_name: String,
    /// Number of active `WebSocket` connections (event subscribers).
    pub(in crate::server) active_connections: Arc<AtomicUsize>,
    /// Whether the daemon is running in ephemeral mode.
    pub(in crate::server) ephemeral: bool,
    /// Plugin IDs explicitly unloaded by the user (shared with watcher).
    pub(in crate::server) user_unloaded_plugins: Arc<RwLock<HashSet<PluginId>>>,
    /// Workspace root directory (consistent with watcher reload path).
    pub(in crate::server) workspace_root: PathBuf,
}

#[jsonrpsee::core::async_trait]
impl AstridRpcServer for RpcImpl {
    async fn create_session(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<SessionInfo, ErrorObjectOwned> {
        self.create_session_impl(workspace_path).await
    }

    async fn resume_session(&self, session_id: SessionId) -> Result<SessionInfo, ErrorObjectOwned> {
        self.resume_session_impl(session_id).await
    }

    async fn send_input(
        &self,
        session_id: SessionId,
        input: String,
    ) -> Result<(), ErrorObjectOwned> {
        self.send_input_impl(session_id, input).await
    }

    async fn approval_response(
        &self,
        session_id: SessionId,
        request_id: String,
        decision: ApprovalDecision,
    ) -> Result<(), ErrorObjectOwned> {
        self.approval_response_impl(session_id, request_id, decision)
            .await
    }

    async fn elicitation_response(
        &self,
        session_id: SessionId,
        request_id: String,
        response: ElicitationResponse,
    ) -> Result<(), ErrorObjectOwned> {
        self.elicitation_response_impl(session_id, request_id, response)
            .await
    }

    async fn list_sessions(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<Vec<SessionInfo>, ErrorObjectOwned> {
        self.list_sessions_impl(workspace_path).await
    }

    async fn end_session(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned> {
        self.end_session_impl(session_id).await
    }

    async fn status(&self) -> Result<DaemonStatus, ErrorObjectOwned> {
        self.status_impl().await
    }

    async fn list_servers(&self) -> Result<Vec<McpServerInfo>, ErrorObjectOwned> {
        self.list_servers_impl().await
    }

    async fn start_server(&self, name: String) -> Result<(), ErrorObjectOwned> {
        self.start_server_impl(name).await
    }

    async fn stop_server(&self, name: String) -> Result<(), ErrorObjectOwned> {
        self.stop_server_impl(name).await
    }

    async fn list_tools(&self) -> Result<Vec<ToolInfo>, ErrorObjectOwned> {
        self.list_tools_impl().await
    }

    async fn shutdown(&self) -> Result<(), ErrorObjectOwned> {
        self.shutdown_impl();
        Ok(())
    }

    async fn session_budget(&self, session_id: SessionId) -> Result<BudgetInfo, ErrorObjectOwned> {
        self.session_budget_impl(session_id).await
    }

    async fn session_allowances(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<AllowanceInfo>, ErrorObjectOwned> {
        self.session_allowances_impl(session_id).await
    }

    async fn session_audit(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<AuditEntryInfo>, ErrorObjectOwned> {
        self.session_audit_impl(&session_id, limit)
    }

    async fn save_session(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned> {
        self.save_session_impl(session_id).await
    }

    async fn list_plugins(&self) -> Result<Vec<PluginInfo>, ErrorObjectOwned> {
        self.list_plugins_impl().await
    }

    async fn load_plugin(&self, plugin_id: String) -> Result<PluginInfo, ErrorObjectOwned> {
        self.load_plugin_impl(plugin_id).await
    }

    async fn unload_plugin(&self, plugin_id: String) -> Result<(), ErrorObjectOwned> {
        self.unload_plugin_impl(plugin_id).await
    }

    async fn cancel_turn(&self, session_id: SessionId) -> Result<(), ErrorObjectOwned> {
        self.cancel_turn_impl(session_id).await
    }

    async fn subscribe_events(
        &self,
        pending: PendingSubscriptionSink,
        session_id: SessionId,
    ) -> jsonrpsee::core::SubscriptionResult {
        self.subscribe_events_impl(pending, session_id).await
    }
}
