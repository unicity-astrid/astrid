//! Plugin tool trait.
//!
//! Mirrors the `BuiltinTool` trait from `astrid-tools` but with dynamic
//! (non-`'static`) names and a plugin-specific context.

use async_trait::async_trait;
use serde_json::Value;

use crate::context::PluginToolContext;
use crate::error::PluginResult;

/// A tool provided by a plugin.
///
/// Similar to `BuiltinTool` but returns `&str` instead of `&'static str`
/// because plugin tool names are loaded at runtime. Uses `PluginToolContext`
/// instead of `ToolContext` to provide scoped KV storage and restrict access.
#[async_trait]
pub trait PluginTool: Send + Sync {
    /// Tool name (unique within the plugin).
    ///
    /// The fully qualified name exposed to the LLM is
    /// `plugin:{plugin_id}:{tool_name}`.
    fn name(&self) -> &str;

    /// Human-readable description for the LLM.
    fn description(&self) -> &str;

    /// JSON schema for tool input parameters.
    fn input_schema(&self) -> Value;

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: Value, ctx: &PluginToolContext) -> PluginResult<String>;
}

impl std::fmt::Debug for dyn PluginTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginTool")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}
