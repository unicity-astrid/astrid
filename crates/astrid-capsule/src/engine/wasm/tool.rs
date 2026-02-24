//! WASM capsule tool — wraps a single tool exported by a WASM guest.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::context::CapsuleToolContext;
use crate::error::{CapsuleError, CapsuleResult};
use crate::tool::CapsuleTool;

#[derive(serde::Serialize)]
struct __AstridToolRequest {
    name: String,
    arguments: Vec<u8>,
}

/// A tool backed by a WASM capsule's `astrid_tool_call` export.
pub struct WasmCapsuleTool {
    name: String,
    description: String,
    input_schema: Value,
    plugin: Arc<Mutex<extism::Plugin>>,
}

impl WasmCapsuleTool {
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
impl CapsuleTool for WasmCapsuleTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    async fn execute(&self, args: Value, _ctx: &CapsuleToolContext) -> CapsuleResult<String> {
        let args_bytes = serde_json::to_vec(&args).map_err(|e| {
            CapsuleError::ExecutionFailed(format!("failed to serialize args: {e}"))
        })?;

        let tool_input = __AstridToolRequest {
            name: self.name.clone(),
            arguments: args_bytes,
        };

        let input_json = serde_json::to_vec(&tool_input).map_err(|e| {
            CapsuleError::ExecutionFailed(format!("failed to serialize ToolInput: {e}"))
        })?;

        let result = tokio::task::block_in_place(|| {
            let mut plugin = self.plugin.lock().map_err(|e| {
                CapsuleError::WasmError(format!("plugin lock poisoned: {e}"))
            })?;
            plugin
                .call::<&[u8], Vec<u8>>("astrid_tool_call", &input_json)
                .map_err(|e| CapsuleError::WasmError(format!("astrid_tool_call failed: {e:?}")))
        })?;

        let output_str = String::from_utf8_lossy(&result).into_owned();
        Ok(output_str)
    }
}

impl std::fmt::Debug for WasmCapsuleTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmCapsuleTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}