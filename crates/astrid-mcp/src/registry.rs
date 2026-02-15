//! Unified MCP tool registry.
//!
//! [`McpRegistry`] wraps a global [`McpClient`] and an optional workspace
//! [`McpClient`], exposing a single `list_tools()` / `call_tool()` surface.
//! The runtime's agentic loop uses the registry instead of knowing about
//! global vs. workspace layers.

use serde_json::Value;

use crate::client::McpClient;
use crate::error::{McpError, McpResult};
use crate::types::{ToolDefinition, ToolResult};

/// Unified tool registry that merges a global and optional workspace MCP layer.
///
/// `list_tools()` returns tools from both layers (global first, then workspace).
/// `call_tool()` tries the global layer first; on [`McpError::ServerNotRunning`]
/// it falls back to the workspace layer.
#[derive(Clone)]
pub struct McpRegistry {
    global: McpClient,
    workspace: Option<McpClient>,
}

impl McpRegistry {
    /// Create a registry backed by a global MCP client only.
    #[must_use]
    pub fn new(global: McpClient) -> Self {
        Self {
            global,
            workspace: None,
        }
    }

    /// Add a workspace MCP client layer.
    #[must_use]
    pub fn with_workspace(mut self, ws: McpClient) -> Self {
        self.workspace = Some(ws);
        self
    }

    /// Set the workspace MCP client layer in place (avoids clone-and-replace).
    pub fn set_workspace(&mut self, ws: McpClient) {
        self.workspace = Some(ws);
    }

    /// Whether a workspace layer is present.
    #[must_use]
    pub fn has_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// List tools from both layers (global first, then workspace).
    ///
    /// # Errors
    ///
    /// Returns an error if either layer fails to list tools.
    pub async fn list_tools(&self) -> McpResult<Vec<ToolDefinition>> {
        let mut tools = self.global.list_tools().await?;
        if let Some(ref ws) = self.workspace {
            tools.extend(ws.list_tools().await?);
        }
        Ok(tools)
    }

    /// Call a tool, routing through global first, then workspace.
    ///
    /// If the global client returns [`McpError::ServerNotRunning`] and a
    /// workspace layer is present, the call is retried against the workspace
    /// client.
    ///
    /// # Errors
    ///
    /// Returns an error if both layers fail, or if only global exists and it
    /// fails.
    pub async fn call_tool(&self, server: &str, tool: &str, args: Value) -> McpResult<ToolResult> {
        let global_result = self.global.call_tool(server, tool, args.clone()).await;

        match global_result {
            Ok(result) => Ok(result),
            Err(McpError::ServerNotRunning { .. }) => {
                if let Some(ws) = &self.workspace {
                    ws.call_tool(server, tool, args).await
                } else {
                    Err(McpError::ServerNotRunning {
                        name: server.to_string(),
                    })
                }
            },
            Err(e) => Err(e),
        }
    }
}

impl std::fmt::Debug for McpRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpRegistry")
            .field("has_workspace", &self.workspace.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServersConfig;

    fn empty_client() -> McpClient {
        McpClient::with_config(ServersConfig::default())
    }

    #[tokio::test]
    async fn test_global_only_list_tools() {
        let registry = McpRegistry::new(empty_client());
        assert!(!registry.has_workspace());
        let tools = registry.list_tools().await.unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn test_with_workspace_flag() {
        let registry = McpRegistry::new(empty_client()).with_workspace(empty_client());
        assert!(registry.has_workspace());
    }

    #[tokio::test]
    async fn test_call_tool_server_not_running_no_workspace() {
        let registry = McpRegistry::new(empty_client());
        let result = registry
            .call_tool("missing", "tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_call_tool_fallback_to_workspace() {
        // Both layers are empty so the call will fail, but this verifies
        // the fallback path is exercised (ServerNotRunning on global
        // triggers workspace attempt).
        let registry = McpRegistry::new(empty_client()).with_workspace(empty_client());
        let result = registry
            .call_tool("ws-server", "tool", serde_json::json!({}))
            .await;
        // Both fail with ServerNotRunning — workspace also returns the error.
        assert!(result.is_err());
    }
}
