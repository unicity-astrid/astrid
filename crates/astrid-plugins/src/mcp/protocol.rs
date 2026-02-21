//! MCP protocol communication abstractions.

use rmcp::model::{CallToolRequestParams, ClientNotification, CustomNotification};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde_json::Value;

use crate::error::{PluginError, PluginResult};

/// Trait to define how Astrid communicates with an MCP Server process.
#[async_trait::async_trait]
pub trait McpProtocolConnection: Send + Sync {
    /// Send a custom hook event to the server
    async fn send_hook_event(&self, event: astrid_core::HookEvent, data: Value);

    /// Get the underlying peer to call tools
    fn peer(&self) -> Option<Peer<RoleClient>>;

    /// Close the connection gracefully
    async fn close(&mut self, timeout: std::time::Duration) -> PluginResult<()>;
}
