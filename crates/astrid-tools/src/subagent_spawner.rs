//! Sub-agent spawner trait for dependency inversion.
//!
//! `astrid-tools` defines this trait; `astrid-runtime` implements it.
//! This avoids a circular dependency between the two crates.

use std::time::Duration;

/// Request to spawn a sub-agent.
#[derive(Debug, Clone)]
pub struct SubAgentRequest {
    /// Short description of the task (shown in status/logs).
    pub description: String,
    /// Detailed instructions for the sub-agent.
    pub prompt: String,
    /// Optional timeout (falls back to executor default if `None`).
    pub timeout: Option<Duration>,
}

/// Result returned by a completed sub-agent.
#[derive(Debug, Clone)]
pub struct SubAgentResult {
    /// Whether the sub-agent completed successfully.
    pub success: bool,
    /// Output text from the sub-agent (last assistant message).
    pub output: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Number of tool calls the sub-agent made.
    pub tool_calls: usize,
    /// Error message (if `success` is false).
    pub error: Option<String>,
}

/// Trait for spawning sub-agents from built-in tools.
///
/// Implemented by `SubAgentExecutor` in `astrid-runtime`.
/// Injected into `ToolContext` as `Arc<dyn SubAgentSpawner>`.
#[async_trait::async_trait]
pub trait SubAgentSpawner: Send + Sync {
    /// Spawn a sub-agent and wait for its result.
    async fn spawn(&self, request: SubAgentRequest) -> Result<SubAgentResult, String>;
}
