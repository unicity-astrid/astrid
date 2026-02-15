//! Hook execution results and context.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::hook::HookEvent;

/// Result of hook execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum HookResult {
    /// Continue with the operation (no changes).
    #[default]
    Continue,
    /// Continue with modified context.
    ContinueWith {
        /// Modified context values.
        modifications: HashMap<String, serde_json::Value>,
    },
    /// Block the operation.
    Block {
        /// Reason for blocking.
        reason: String,
    },
    /// Ask the user before proceeding.
    Ask {
        /// Question to ask the user.
        question: String,
        /// Default answer if user doesn't respond.
        #[serde(default)]
        default: Option<String>,
    },
}

impl HookResult {
    /// Create a continue result.
    #[must_use]
    pub fn continue_() -> Self {
        Self::Continue
    }

    /// Create a continue-with-modifications result.
    #[must_use]
    pub fn continue_with(modifications: HashMap<String, serde_json::Value>) -> Self {
        Self::ContinueWith { modifications }
    }

    /// Create a block result.
    #[must_use]
    pub fn block(reason: impl Into<String>) -> Self {
        Self::Block {
            reason: reason.into(),
        }
    }

    /// Create an ask result.
    #[must_use]
    pub fn ask(question: impl Into<String>) -> Self {
        Self::Ask {
            question: question.into(),
            default: None,
        }
    }

    /// Check if this result blocks the operation.
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::Block { .. })
    }

    /// Check if this result requires user interaction.
    #[must_use]
    pub fn requires_interaction(&self) -> bool {
        matches!(self, Self::Ask { .. })
    }
}

/// Context provided to hooks during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// Unique identifier for this hook invocation.
    pub invocation_id: Uuid,
    /// The event that triggered the hook.
    pub event: HookEvent,
    /// Session ID if available.
    #[serde(default)]
    pub session_id: Option<Uuid>,
    /// User ID if available.
    #[serde(default)]
    pub user_id: Option<Uuid>,
    /// Timestamp of the event.
    pub timestamp: DateTime<Utc>,
    /// Event-specific data.
    #[serde(default)]
    pub data: HashMap<String, serde_json::Value>,
    /// Previous hook results in the chain.
    #[serde(default)]
    pub previous_results: Vec<HookResult>,
}

impl HookContext {
    /// Create a new hook context.
    #[must_use]
    pub fn new(event: HookEvent) -> Self {
        Self {
            invocation_id: Uuid::new_v4(),
            event,
            session_id: None,
            user_id: None,
            timestamp: Utc::now(),
            data: HashMap::new(),
            previous_results: Vec::new(),
        }
    }

    /// Set the session ID.
    #[must_use]
    pub fn with_session(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Set the user ID.
    #[must_use]
    pub fn with_user(mut self, user_id: Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Add data to the context.
    #[must_use]
    pub fn with_data(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.data.insert(key.into(), value);
        self
    }

    /// Add a previous hook result.
    pub fn add_previous_result(&mut self, result: HookResult) {
        self.previous_results.push(result);
    }

    /// Get a data value.
    #[must_use]
    pub fn get_data(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }

    /// Get a data value as a specific type.
    #[must_use]
    pub fn get_data_as<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Option<T> {
        self.data
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Check if any previous hook blocked.
    #[must_use]
    pub fn was_blocked(&self) -> bool {
        self.previous_results.iter().any(HookResult::is_blocking)
    }

    /// Convert context to JSON for passing to handlers.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Convert context to environment variables.
    #[must_use]
    pub fn to_env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        env.insert("ASTRID_HOOK_ID".to_string(), self.invocation_id.to_string());
        env.insert("ASTRID_HOOK_EVENT".to_string(), self.event.to_string());
        env.insert(
            "ASTRID_HOOK_TIMESTAMP".to_string(),
            self.timestamp.to_rfc3339(),
        );

        if let Some(session_id) = &self.session_id {
            env.insert("ASTRID_SESSION_ID".to_string(), session_id.to_string());
        }

        if let Some(user_id) = &self.user_id {
            env.insert("ASTRID_USER_ID".to_string(), user_id.to_string());
        }

        // Add data as JSON
        if !self.data.is_empty()
            && let Ok(json) = serde_json::to_string(&self.data)
        {
            env.insert("ASTRID_HOOK_DATA".to_string(), json);
        }

        env
    }
}

/// Execution metadata for a hook run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookExecution {
    /// Hook ID that was executed.
    pub hook_id: Uuid,
    /// Invocation ID from the context.
    pub invocation_id: Uuid,
    /// When execution started.
    pub started_at: DateTime<Utc>,
    /// When execution completed.
    pub completed_at: DateTime<Utc>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Result of the execution.
    pub result: HookExecutionResult,
}

/// Result of hook execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum HookExecutionResult {
    /// Hook executed successfully.
    Success {
        /// The hook's result.
        result: HookResult,
        /// Stdout output if applicable.
        #[serde(default)]
        stdout: Option<String>,
    },
    /// Hook failed to execute.
    Failure {
        /// Error message.
        error: String,
        /// Stderr output if applicable.
        #[serde(default)]
        stderr: Option<String>,
    },
    /// Hook execution timed out.
    Timeout {
        /// Timeout duration in seconds.
        timeout_secs: u64,
    },
    /// Hook was skipped (disabled or matcher didn't match).
    Skipped {
        /// Reason for skipping.
        reason: String,
    },
}

impl HookExecutionResult {
    /// Check if execution was successful.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Get the hook result if successful.
    #[must_use]
    pub fn hook_result(&self) -> Option<&HookResult> {
        match self {
            Self::Success { result, .. } => Some(result),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_result_continue() {
        let result = HookResult::continue_();
        assert!(!result.is_blocking());
        assert!(!result.requires_interaction());
    }

    #[test]
    fn test_hook_result_block() {
        let result = HookResult::block("Policy violation");
        assert!(result.is_blocking());
    }

    #[test]
    fn test_hook_result_ask() {
        let result = HookResult::ask("Are you sure?");
        assert!(result.requires_interaction());
    }

    #[test]
    fn test_hook_context_creation() {
        let session_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();

        let ctx = HookContext::new(HookEvent::PreToolCall)
            .with_session(session_id)
            .with_user(user_id)
            .with_data("tool_name", serde_json::json!("read_file"));

        assert_eq!(ctx.event, HookEvent::PreToolCall);
        assert_eq!(ctx.session_id, Some(session_id));
        assert_eq!(ctx.user_id, Some(user_id));
        assert!(ctx.get_data("tool_name").is_some());
    }

    #[test]
    fn test_hook_context_env_vars() {
        let ctx = HookContext::new(HookEvent::SessionStart);
        let env = ctx.to_env_vars();

        assert!(env.contains_key("ASTRID_HOOK_ID"));
        assert_eq!(
            env.get("ASTRID_HOOK_EVENT"),
            Some(&"session_start".to_string())
        );
    }

    #[test]
    fn test_hook_execution_result() {
        let success = HookExecutionResult::Success {
            result: HookResult::Continue,
            stdout: Some("ok".to_string()),
        };
        assert!(success.is_success());
        assert!(success.hook_result().is_some());

        let failure = HookExecutionResult::Failure {
            error: "command failed".to_string(),
            stderr: None,
        };
        assert!(!failure.is_success());
        assert!(failure.hook_result().is_none());
    }
}
