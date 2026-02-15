//! Hook handler implementations.
//!
//! Each handler type has its own module with the execution logic.

pub mod agent;
pub mod command;
pub mod http;
pub mod wasm;

pub use agent::AgentHandler;
pub use command::CommandHandler;
pub use http::HttpHandler;
pub use wasm::WasmHandler;

use crate::hook::HookHandler;
use crate::result::{HookContext, HookExecutionResult, HookResult};
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during hook handler execution.
#[derive(Debug, Error)]
pub enum HandlerError {
    /// Command execution failed.
    #[error("command execution failed: {0}")]
    CommandFailed(String),

    /// HTTP request failed.
    #[error("HTTP request failed: {0}")]
    HttpFailed(String),

    /// WASM execution failed.
    #[error("WASM execution failed: {0}")]
    WasmFailed(String),

    /// Agent execution failed.
    #[error("agent execution failed: {0}")]
    AgentFailed(String),

    /// Handler timed out.
    #[error("handler timed out after {0:?}")]
    Timeout(Duration),

    /// Handler is not implemented (stubbed).
    #[error("handler not implemented: {0}")]
    NotImplemented(String),

    /// Invalid handler configuration.
    #[error("invalid handler configuration: {0}")]
    InvalidConfiguration(String),

    /// Failed to parse handler output.
    #[error("failed to parse handler output: {0}")]
    ParseError(String),

    /// IO error.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Result type for handler operations.
pub type HandlerResult<T> = Result<T, HandlerError>;

/// Trait for executing hook handlers.
#[allow(async_fn_in_trait)]
pub trait HandlerExecutor: Send + Sync {
    /// Execute the handler with the given context.
    async fn execute(
        &self,
        handler: &HookHandler,
        context: &HookContext,
        timeout: Duration,
    ) -> HandlerResult<HookExecutionResult>;
}

/// Parse hook result from handler output.
///
/// The output can be:
/// - JSON object with `action` field
/// - Simple string: "continue", "block:<reason>", "ask:<question>"
/// - Empty or whitespace: defaults to Continue
///
/// # Errors
///
/// Returns an error if the output is invalid JSON.
pub fn parse_hook_result(output: &str) -> HandlerResult<HookResult> {
    let trimmed = output.trim();

    if trimmed.is_empty() {
        return Ok(HookResult::Continue);
    }

    // Try JSON first
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed)
            .map_err(|e| HandlerError::ParseError(format!("invalid JSON: {e}")));
    }

    // Parse simple string format
    let lower = trimmed.to_lowercase();
    if lower == "continue" {
        return Ok(HookResult::Continue);
    }

    if let Some(reason) = lower.strip_prefix("block:") {
        return Ok(HookResult::block(reason.trim()));
    }

    if let Some(question) = lower.strip_prefix("ask:") {
        return Ok(HookResult::ask(question.trim()));
    }

    // Unknown format, treat as continue
    Ok(HookResult::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hook_result_empty() {
        let result = parse_hook_result("").unwrap();
        assert!(matches!(result, HookResult::Continue));
    }

    #[test]
    fn test_parse_hook_result_continue() {
        let result = parse_hook_result("continue").unwrap();
        assert!(matches!(result, HookResult::Continue));
    }

    #[test]
    fn test_parse_hook_result_block() {
        let result = parse_hook_result("block: Policy violation").unwrap();
        assert!(matches!(result, HookResult::Block { reason } if reason == "policy violation"));
    }

    #[test]
    fn test_parse_hook_result_ask() {
        let result = parse_hook_result("ask: Are you sure?").unwrap();
        assert!(matches!(result, HookResult::Ask { question, .. } if question == "are you sure?"));
    }

    #[test]
    fn test_parse_hook_result_json() {
        let json = r#"{"action": "block", "reason": "Not allowed"}"#;
        let result = parse_hook_result(json).unwrap();
        assert!(matches!(result, HookResult::Block { .. }));
    }
}
