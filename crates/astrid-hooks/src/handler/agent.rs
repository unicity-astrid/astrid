//! Agent hook handler (stubbed for Phase 3).
//!
//! This module provides the interface for LLM-based hook handlers.
//! The actual implementation will be added in Phase 3 when the LLM
//! integration is more mature.

use std::time::Duration;
use tracing::warn;

use super::{HandlerError, HandlerResult};
use crate::hook::HookHandler;
use crate::result::{HookContext, HookExecutionResult};

/// Handler for LLM-based agents (stubbed).
///
/// This handler will invoke an LLM to process hook events
/// in Phase 3. For now, it returns a stub response.
#[derive(Debug, Clone, Default)]
pub struct AgentHandler;

impl AgentHandler {
    /// Create a new agent handler.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Execute an agent handler (stubbed).
    ///
    /// # Errors
    ///
    /// Returns an error if the handler configuration is invalid.
    #[allow(clippy::unused_async)]
    pub async fn execute(
        &self,
        handler: &HookHandler,
        _context: &HookContext,
        _timeout: Duration,
    ) -> HandlerResult<HookExecutionResult> {
        let HookHandler::Agent {
            prompt_template,
            model,
            max_tokens,
        } = handler
        else {
            return Err(HandlerError::InvalidConfiguration(
                "expected Agent handler".to_string(),
            ));
        };

        warn!(
            prompt_template = %prompt_template,
            model = ?model,
            max_tokens = ?max_tokens,
            "Agent handler is stubbed - will be implemented in Phase 3"
        );

        // For now, return a skipped result
        Ok(HookExecutionResult::Skipped {
            reason: format!(
                "Agent handlers are not yet implemented (model: {})",
                model.as_deref().unwrap_or("default")
            ),
        })
    }

    /// Check if the agent runtime is available.
    ///
    /// Always returns `false` until Phase 3 implementation.
    #[must_use]
    pub fn is_available() -> bool {
        false
    }
}

/// Configuration for agent execution (for Phase 3).
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Default model to use.
    pub default_model: String,
    /// Maximum tokens for responses.
    pub max_tokens: u32,
    /// Temperature for sampling.
    pub temperature: f64,
    /// System prompt prefix.
    pub system_prompt: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_model: "claude-3-haiku".to_string(),
            max_tokens: 1024,
            temperature: 0.0,
            system_prompt: "You are a hook handler for the Astrid agent runtime. \
                Analyze the event and return a JSON response with an 'action' field \
                (continue, block, or ask) and appropriate additional fields."
                .to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookEvent;

    #[tokio::test]
    async fn test_agent_handler_stubbed() {
        let handler = AgentHandler::new();
        let hook_handler = HookHandler::Agent {
            prompt_template: "Analyze this event: {{event}}".to_string(),
            model: Some("claude-3-haiku".to_string()),
            max_tokens: Some(512),
        };
        let context = HookContext::new(HookEvent::PreToolCall);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(30))
            .await
            .unwrap();

        assert!(matches!(result, HookExecutionResult::Skipped { .. }));
    }

    #[test]
    fn test_agent_not_available() {
        assert!(!AgentHandler::is_available());
    }

    #[test]
    fn test_agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.max_tokens, 1024);
        assert_eq!(config.temperature, 0.0);
    }
}
