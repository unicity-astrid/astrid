//! WASM plugin tool — wraps a single tool exported by a WASM guest.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use astrid_core::plugin_abi::{ToolInput, ToolOutput};

use crate::context::PluginToolContext;
use crate::error::{PluginError, PluginResult};
use crate::tool::PluginTool;

/// A tool backed by a WASM plugin's `execute-tool` export.
///
/// Multiple `WasmPluginTool` instances share the same `Arc<Mutex<extism::Plugin>>`
/// since WASM execution is inherently single-threaded.
pub struct WasmPluginTool {
    /// Tool name (unique within the plugin).
    name: String,
    /// Human-readable description.
    description: String,
    /// JSON schema for input parameters.
    input_schema: Value,
    /// Shared Extism plugin instance (all tools share this).
    plugin: Arc<Mutex<extism::Plugin>>,
}

impl WasmPluginTool {
    /// Create a new WASM plugin tool.
    pub(crate) fn new(
        name: String,
        description: String,
        input_schema: Value,
        plugin: Arc<Mutex<extism::Plugin>>,
    ) -> Self {
        Self {
            name,
            description,
            input_schema,
            plugin,
        }
    }
}

#[async_trait]
impl PluginTool for WasmPluginTool {
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
        let tool_input = ToolInput {
            name: self.name.clone(),
            arguments: serde_json::to_string(&args).map_err(|e| {
                PluginError::ExecutionFailed(format!("failed to serialize args: {e}"))
            })?,
        };

        let input_json = serde_json::to_string(&tool_input).map_err(|e| {
            PluginError::ExecutionFailed(format!("failed to serialize ToolInput: {e}"))
        })?;

        // block_in_place allows blocking in an async context with the multi-threaded runtime.
        let result = tokio::task::block_in_place(|| {
            let mut plugin = self
                .plugin
                .lock()
                .map_err(|e| PluginError::WasmError(format!("plugin lock poisoned: {e}")))?;
            plugin
                .call::<&str, String>("execute-tool", &input_json)
                .map_err(|e| PluginError::WasmError(format!("execute-tool call failed: {e}")))
        })?;

        // Parse the output
        let output: ToolOutput = serde_json::from_str(&result).map_err(|e| {
            PluginError::ExecutionFailed(format!("failed to parse ToolOutput: {e}"))
        })?;

        if output.is_error {
            Err(PluginError::ExecutionFailed(output.content))
        } else {
            Ok(output.content)
        }
    }
}

impl std::fmt::Debug for WasmPluginTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmPluginTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

// Note: WasmPluginTool unit tests requiring a real Extism Plugin are deferred
// to integration tests with WASM fixtures. The struct itself is tested through
// WasmPlugin lifecycle tests.
