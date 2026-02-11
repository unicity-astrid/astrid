//! Task tool — spawns a sub-agent to handle a scoped task autonomously.

use crate::subagent_spawner::SubAgentRequest;
use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use serde_json::Value;
use std::time::Duration;

/// Maximum allowed timeout for sub-agents (50 minutes).
const MAX_TIMEOUT_SECS: u64 = 3000;

/// Tool for spawning sub-agent tasks.
pub struct TaskTool;

#[async_trait::async_trait]
impl BuiltinTool for TaskTool {
    fn name(&self) -> &'static str {
        "task"
    }

    fn description(&self) -> &'static str {
        "Spawns a sub-agent to handle a complex, multi-step task autonomously. \
         The sub-agent works within inherited capability bounds and returns a result."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "A short description of the task (3-5 words)"
                },
                "prompt": {
                    "type": "string",
                    "description": "Detailed instructions for the sub-agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 300)"
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'description'".into()))?
            .to_string();

        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'prompt'".into()))?
            .to_string();

        let timeout = args
            .get("timeout_secs")
            .and_then(serde_json::Value::as_u64)
            .map(|s| Duration::from_secs(s.min(MAX_TIMEOUT_SECS)));

        let spawner = ctx.subagent_spawner().await.ok_or_else(|| {
            ToolError::ExecutionFailed("Sub-agent spawning is not available in this context".into())
        })?;

        let request = SubAgentRequest {
            description,
            prompt,
            timeout,
        };

        match spawner.spawn(request).await {
            Ok(result) => {
                if result.success {
                    Ok(result.output)
                } else {
                    let error_msg = result
                        .error
                        .unwrap_or_else(|| "sub-agent failed".to_string());
                    Err(ToolError::ExecutionFailed(format!(
                        "Sub-agent failed: {error_msg}"
                    )))
                }
            },
            Err(e) => Err(ToolError::ExecutionFailed(format!(
                "Failed to spawn sub-agent: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagent_spawner::{SubAgentResult, SubAgentSpawner};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_task_without_spawner_returns_error() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let result = TaskTool
            .execute(
                serde_json::json!({
                    "description": "test",
                    "prompt": "do something"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not available in this context")
        );
    }

    #[tokio::test]
    async fn test_task_missing_description() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let result = TaskTool
            .execute(serde_json::json!({"prompt": "do something"}), &ctx)
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing 'description'")
        );
    }

    #[tokio::test]
    async fn test_task_missing_prompt() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let result = TaskTool
            .execute(serde_json::json!({"description": "test"}), &ctx)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'prompt'"));
    }

    struct MockSpawner {
        response: SubAgentResult,
    }

    #[async_trait::async_trait]
    impl SubAgentSpawner for MockSpawner {
        async fn spawn(&self, _request: SubAgentRequest) -> Result<SubAgentResult, String> {
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn test_task_with_mock_spawner_success() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let spawner = Arc::new(MockSpawner {
            response: SubAgentResult {
                success: true,
                output: "Task completed successfully".into(),
                duration_ms: 1000,
                tool_calls: 3,
                error: None,
            },
        });
        ctx.set_subagent_spawner(Some(spawner)).await;

        let result = TaskTool
            .execute(
                serde_json::json!({
                    "description": "test task",
                    "prompt": "do the thing"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Task completed successfully");
    }

    #[tokio::test]
    async fn test_task_with_mock_spawner_failure() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let spawner = Arc::new(MockSpawner {
            response: SubAgentResult {
                success: false,
                output: String::new(),
                duration_ms: 500,
                tool_calls: 1,
                error: Some("ran out of budget".into()),
            },
        });
        ctx.set_subagent_spawner(Some(spawner)).await;

        let result = TaskTool
            .execute(
                serde_json::json!({
                    "description": "failing task",
                    "prompt": "do something expensive"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("ran out of budget")
        );
    }

    #[tokio::test]
    async fn test_task_timeout_clamped_to_max() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let spawner = Arc::new(MockSpawner {
            response: SubAgentResult {
                success: true,
                output: "done".into(),
                duration_ms: 100,
                tool_calls: 0,
                error: None,
            },
        });
        ctx.set_subagent_spawner(Some(spawner)).await;

        // Pass an absurdly large timeout — should be clamped to MAX_TIMEOUT_SECS
        let result = TaskTool
            .execute(
                serde_json::json!({
                    "description": "test",
                    "prompt": "do something",
                    "timeout_secs": 999_999_999_u64
                }),
                &ctx,
            )
            .await;

        // The tool should succeed (clamped timeout doesn't affect mock spawner)
        assert!(result.is_ok());
    }

    struct ErrorSpawner;

    #[async_trait::async_trait]
    impl SubAgentSpawner for ErrorSpawner {
        async fn spawn(&self, _request: SubAgentRequest) -> Result<SubAgentResult, String> {
            Err("maximum concurrent subagents reached".into())
        }
    }

    #[tokio::test]
    async fn test_task_spawn_error() {
        let ctx = ToolContext::new(std::env::temp_dir());
        ctx.set_subagent_spawner(Some(Arc::new(ErrorSpawner))).await;

        let result = TaskTool
            .execute(
                serde_json::json!({
                    "description": "test",
                    "prompt": "do something"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("maximum concurrent subagents reached")
        );
    }
}
