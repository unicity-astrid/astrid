use async_trait::async_trait;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{Peer, RoleClient};
use serde_json::Value;
use std::borrow::Cow;

use crate::context::PluginToolContext;
use crate::error::{PluginError, PluginResult};
use crate::tool::PluginTool;
use astrid_mcp::ToolResult;

/// A tool provided by an MCP server, wrapped as a [`PluginTool`].
///
/// Tool calls are forwarded directly to the MCP server via the stored
/// [`Peer`] handle. Security is enforced at the runtime layer (before
/// `execute()` is called), not here.
pub(crate) struct McpPluginTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
    #[allow(dead_code)]
    pub(crate) server_name: String,
    pub(crate) peer: Peer<RoleClient>,
}

#[async_trait]
impl PluginTool for McpPluginTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    async fn execute(&self, args: Value, _ctx: &PluginToolContext) -> PluginResult<String> {
        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), other);
                Some(map)
            },
        };

        let params = CallToolRequestParams {
            meta: None,
            name: Cow::Owned(self.name.clone()),
            arguments,
            task: None,
        };

        let result = self
            .peer
            .call_tool(params)
            .await
            .map_err(|e| PluginError::ExecutionFailed(format!("MCP tool call failed: {e}")))?;

        // Convert to our ToolResult and extract text content
        let tool_result = ToolResult::from(result);
        if tool_result.is_error {
            return Err(PluginError::ExecutionFailed(
                tool_result
                    .error
                    .unwrap_or_else(|| "Unknown MCP tool error".into()),
            ));
        }

        Ok(tool_result.text_content())
    }
}
