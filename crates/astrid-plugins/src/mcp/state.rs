//! MCP state machine and connection management.

use astrid_mcp::AstridClientHandler;
use rmcp::service::{Peer, RoleClient, RunningService};
use serde_json::Value;
use std::time::Duration;

use super::protocol::McpProtocolConnection;
use crate::error::PluginResult;

/// Type alias for the running MCP service backing a plugin.
pub type PluginMcpService = RunningService<RoleClient, AstridClientHandler>;

/// Represents an active connection to an MCP plugin server.
pub struct McpConnection {
    plugin_id: crate::plugin::PluginId,
    service: Option<PluginMcpService>,
    peer: Option<Peer<RoleClient>>,
}

impl McpConnection {
    /// Initializes a new `OpenMcpConnection` tracker.
    #[must_use] 
    pub fn new(
        plugin_id: crate::plugin::PluginId,
        service: PluginMcpService,
        peer: Peer<RoleClient>,
    ) -> Self {
        Self {
            plugin_id,
            service: Some(service),
            peer: Some(peer),
        }
    }

    /// Evaluates if the task running the connection loop is still active.
    #[must_use] 
    pub fn is_alive(&self) -> bool {
        self.service.as_ref().is_some_and(|s| !s.is_closed())
    }

    /// Acquires and removes the sender channel to the running MCP service, if still attached.
    pub fn take_service(&mut self) -> Option<PluginMcpService> {
        self.service.take()
    }
}

#[async_trait::async_trait]
impl McpProtocolConnection for McpConnection {
    async fn send_hook_event(&self, event: astrid_core::HookEvent, data: Value) {
        let Some(peer) = &self.peer else {
            return;
        };

        let notification = rmcp::model::CustomNotification::new(
            "notifications/astrid.hookEvent",
            Some(serde_json::json!({
                "event": event.to_string(),
                "data": data,
            })),
        );

        if let Err(e) = peer
            .send_notification(rmcp::model::ClientNotification::CustomNotification(
                notification,
            ))
            .await
        {
            tracing::warn!(
                plugin_id = %self.plugin_id,
                event = %event,
                error = %e,
                "Failed to send hook event to plugin MCP server"
            );
        }
    }

    fn peer(&self) -> Option<Peer<RoleClient>> {
        self.peer.clone()
    }

    async fn close(&mut self, timeout: Duration) -> PluginResult<()> {
        self.peer = None;
        if let Some(ref mut service) = self.service {
            match service.close_with_timeout(timeout).await {
                Ok(Some(reason)) => {
                    tracing::info!(plugin_id = %self.plugin_id, ?reason, "Plugin MCP session closed gracefully");
                },
                Ok(None) => {
                    tracing::warn!(plugin_id = %self.plugin_id, "Plugin MCP session close timed out; dropping");
                },
                Err(e) => {
                    tracing::error!(plugin_id = %self.plugin_id, error = %e, "Plugin MCP session close join error");
                },
            }
        }
        self.service = None;
        Ok(())
    }
}
