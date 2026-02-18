//! MCP server management RPC method implementations.

use std::sync::atomic::Ordering;

use astrid_plugins::PluginState;
use jsonrpsee::types::ErrorObjectOwned;
use tracing::info;

use super::RpcImpl;
use crate::rpc::{DaemonStatus, McpServerInfo, error_codes};

impl RpcImpl {
    pub(super) async fn list_servers_impl(&self) -> Result<Vec<McpServerInfo>, ErrorObjectOwned> {
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

    pub(super) async fn start_server_impl(&self, name: String) -> Result<(), ErrorObjectOwned> {
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

    pub(super) async fn stop_server_impl(&self, name: String) -> Result<(), ErrorObjectOwned> {
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

    pub(super) async fn status_impl(&self) -> Result<DaemonStatus, ErrorObjectOwned> {
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
}
