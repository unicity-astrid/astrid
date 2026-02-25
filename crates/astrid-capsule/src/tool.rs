//! Capsule tool trait.

use async_trait::async_trait;
use serde_json::Value;

use crate::context::CapsuleToolContext;
use crate::error::CapsuleResult;

/// A tool provided by a capsule.
#[async_trait]
pub trait CapsuleTool: Send + Sync {
    /// Tool name (unique within the capsule).
    fn name(&self) -> &str;

    /// Human-readable description for the LLM.
    fn description(&self) -> &str;

    /// JSON schema for tool input parameters.
    fn input_schema(&self) -> Value;

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: Value, ctx: &CapsuleToolContext) -> CapsuleResult<String>;
}

impl std::fmt::Debug for dyn CapsuleTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapsuleTool")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}
