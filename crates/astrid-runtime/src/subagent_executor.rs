//! Sub-agent executor — implements `SubAgentSpawner` using the runtime's agentic loop.

use crate::AgentRuntime;
use crate::session::AgentSession;
use crate::subagent::{SubAgentId, SubAgentPool};

use astrid_audit::{AuditAction, AuditOutcome, AuthorizationProof};
use astrid_core::{Frontend, SessionId};
use astrid_llm::{LlmProvider, Message, MessageContent, MessageRole};
use astrid_tools::{SubAgentRequest, SubAgentResult, SubAgentSpawner};

use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Default sub-agent timeout (5 minutes).
pub const DEFAULT_SUBAGENT_TIMEOUT: Duration = Duration::from_secs(300);

/// Executor that spawns sub-agents through the runtime's agentic loop.
///
/// Created per-turn and injected into `ToolContext` as `Arc<dyn SubAgentSpawner>`.
pub struct SubAgentExecutor<P: LlmProvider, F: Frontend + 'static> {
    /// The runtime (owns LLM, MCP, audit, etc.).
    runtime: Arc<AgentRuntime<P>>,
    /// The shared sub-agent pool (enforces concurrency/depth).
    pool: Arc<SubAgentPool>,
    /// The frontend for this turn (for approval forwarding).
    frontend: Arc<F>,
    /// Parent user ID (inherited by child sessions).
    parent_user_id: [u8; 8],
    /// Parent sub-agent ID (if this executor is itself inside a sub-agent).
    parent_subagent_id: Option<SubAgentId>,
    /// Parent session ID (for audit linkage).
    parent_session_id: SessionId,
    /// Parent's allowance store (shared with child for permission inheritance).
    parent_allowance_store: Arc<astrid_approval::AllowanceStore>,
    /// Parent's capability store (shared with child for capability inheritance).
    parent_capabilities: Arc<astrid_capabilities::CapabilityStore>,
    /// Parent's budget tracker (shared with child so spend is visible bidirectionally).
    parent_budget_tracker: Arc<astrid_approval::budget::BudgetTracker>,
    /// Default timeout for sub-agents.
    default_timeout: Duration,
}

impl<P: LlmProvider, F: Frontend + 'static> SubAgentExecutor<P, F> {
    /// Create a new sub-agent executor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime: Arc<AgentRuntime<P>>,
        pool: Arc<SubAgentPool>,
        frontend: Arc<F>,
        parent_user_id: [u8; 8],
        parent_subagent_id: Option<SubAgentId>,
        parent_session_id: SessionId,
        parent_allowance_store: Arc<astrid_approval::AllowanceStore>,
        parent_capabilities: Arc<astrid_capabilities::CapabilityStore>,
        parent_budget_tracker: Arc<astrid_approval::budget::BudgetTracker>,
        default_timeout: Duration,
    ) -> Self {
        Self {
            runtime,
            pool,
            frontend,
            parent_user_id,
            parent_subagent_id,
            parent_session_id,
            parent_allowance_store,
            parent_capabilities,
            parent_budget_tracker,
            default_timeout,
        }
    }
}

#[async_trait::async_trait]
impl<P: LlmProvider + 'static, F: Frontend + 'static> SubAgentSpawner for SubAgentExecutor<P, F> {
    #[allow(clippy::too_many_lines)]
    async fn spawn(&self, request: SubAgentRequest) -> Result<SubAgentResult, String> {
        let start = std::time::Instant::now();
        let timeout = request.timeout.unwrap_or(self.default_timeout);

        // 1. Acquire a slot in the pool (enforces concurrency + depth)
        let handle = self
            .pool
            .spawn(&request.description, self.parent_subagent_id.clone())
            .await
            .map_err(|e| e.to_string())?;

        let handle_id = handle.id.clone();

        info!(
            subagent_id = %handle.id,
            depth = handle.depth,
            description = %request.description,
            "Sub-agent spawned"
        );

        // 2. Mark as running
        handle.mark_running().await;

        // 3. Create a child session with shared stores from parent
        //
        // Sub-agents inherit the parent's AllowanceStore, CapabilityStore, and BudgetTracker
        // so that project-level permissions and budget are shared. The ApprovalManager and
        // DeferredResolutionStore are fresh per-child (independent approval handler registration
        // and independent deferred queue).
        let session_id = SessionId::new();

        // Truncate description in the system prompt to limit prompt injection surface.
        // The full description is still logged/audited separately.
        let safe_description = if request.description.len() > 200 {
            format!("{}...", &request.description[..200])
        } else {
            request.description.clone()
        };
        let subagent_system_prompt = format!(
            "You are a focused sub-agent. Your task:\n\n{safe_description}\n\n\
             Complete this task and provide a clear, concise result. \
             Do not ask for clarification — work with what you have. \
             When done, provide your final answer as a clear summary.",
        );

        let mut session = AgentSession::with_shared_stores(
            session_id.clone(),
            self.parent_user_id,
            subagent_system_prompt,
            Arc::clone(&self.parent_allowance_store),
            Arc::clone(&self.parent_capabilities),
            Arc::clone(&self.parent_budget_tracker),
        );

        // 4. Audit: sub-agent spawned (parent→child linkage)
        {
            if let Err(e) = self.runtime.audit().append(
                self.parent_session_id.clone(),
                AuditAction::SubAgentSpawned {
                    parent_session_id: self.parent_session_id.0.to_string(),
                    child_session_id: session_id.0.to_string(),
                    description: request.description.clone(),
                },
                AuthorizationProof::System {
                    reason: format!("sub-agent spawned for: {}", request.description),
                },
                AuditOutcome::success(),
            ) {
                warn!(error = %e, "Failed to audit sub-agent spawn linkage");
            }
        }

        // 5. Audit: session started
        {
            if let Err(e) = self.runtime.audit().append(
                session_id.clone(),
                AuditAction::SessionStarted {
                    user_id: self.parent_user_id,
                    frontend: "sub-agent".to_string(),
                },
                AuthorizationProof::System {
                    reason: format!("sub-agent for: {}", request.description),
                },
                AuditOutcome::success(),
            ) {
                warn!(error = %e, "Failed to audit sub-agent session start");
            }
        }

        // 6. Run the agentic loop with timeout + cooperative cancellation
        //
        // `None` = cancelled via token (treated same as timeout — extract partial output).
        // `Some(Ok(Ok(())))` = completed successfully.
        // `Some(Ok(Err(e)))` = runtime error.
        // `Some(Err(_))` = timed out.
        let cancel_token = self.pool.cancellation_token();
        let loop_result = tokio::select! {
            biased;
            () = cancel_token.cancelled() => None,
            result = tokio::time::timeout(
                timeout,
                self.runtime.run_subagent_turn(
                    &mut session,
                    &request.prompt,
                    Arc::clone(&self.frontend),
                    Some(handle_id.clone()),
                ),
            ) => Some(result),
        };

        // 7. Process result
        let tool_call_count = session.metadata.tool_call_count;
        // Sub-agent timeout is at most 5 minutes, so millis always fits in u64.
        #[allow(clippy::cast_possible_truncation)]
        let duration_ms = start.elapsed().as_millis() as u64;

        let result = match loop_result {
            Some(Ok(Ok(()))) => {
                // Extract last assistant message as the output
                let output = extract_last_assistant_text(&session.messages);

                debug!(
                    subagent_id = %handle_id,
                    duration_ms,
                    tool_calls = tool_call_count,
                    output_len = output.len(),
                    "Sub-agent completed successfully"
                );

                handle.complete(&output).await;

                SubAgentResult {
                    success: true,
                    output,
                    duration_ms,
                    tool_calls: tool_call_count,
                    error: None,
                }
            },
            Some(Ok(Err(e))) => {
                let error_msg = e.to_string();
                let partial_output = extract_last_assistant_text(&session.messages);
                warn!(
                    subagent_id = %handle_id,
                    error = %error_msg,
                    partial_output_len = partial_output.len(),
                    duration_ms,
                    "Sub-agent failed"
                );

                handle.fail(&error_msg).await;

                SubAgentResult {
                    success: false,
                    output: partial_output,
                    duration_ms,
                    tool_calls: tool_call_count,
                    error: Some(error_msg),
                }
            },
            Some(Err(_elapsed)) => {
                let partial_output = extract_last_assistant_text(&session.messages);
                warn!(
                    subagent_id = %handle_id,
                    timeout_secs = timeout.as_secs(),
                    partial_output_len = partial_output.len(),
                    duration_ms,
                    "Sub-agent timed out"
                );

                handle.timeout().await;

                SubAgentResult {
                    success: false,
                    output: partial_output,
                    duration_ms,
                    tool_calls: tool_call_count,
                    error: Some(format!(
                        "Sub-agent timed out after {} seconds",
                        timeout.as_secs()
                    )),
                }
            },
            None => {
                // Cooperative cancellation via CancellationToken
                let partial_output = extract_last_assistant_text(&session.messages);
                warn!(
                    subagent_id = %handle_id,
                    partial_output_len = partial_output.len(),
                    duration_ms,
                    "Sub-agent cancelled via token"
                );

                handle.cancel().await;

                SubAgentResult {
                    success: false,
                    output: partial_output,
                    duration_ms,
                    tool_calls: tool_call_count,
                    error: Some("Sub-agent cancelled".to_string()),
                }
            },
        };

        // 8. Release from pool (releases semaphore permit)
        self.pool.release(&handle_id).await;

        // 9. Audit: session ended
        {
            let reason = if result.success {
                "completed".to_string()
            } else {
                result.error.as_deref().unwrap_or("failed").to_string()
            };
            if let Err(e) = self.runtime.audit().append(
                session_id,
                AuditAction::SessionEnded {
                    reason,
                    duration_secs: duration_ms / 1000,
                },
                AuthorizationProof::System {
                    reason: "sub-agent ended".to_string(),
                },
                AuditOutcome::success(),
            ) {
                warn!(error = %e, "Failed to audit sub-agent session end");
            }
        }

        Ok(result)
    }
}

/// Extract the last assistant text message from the session messages.
fn extract_last_assistant_text(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Assistant)
        .and_then(|m| match &m.content {
            MessageContent::Text(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "(sub-agent produced no text output)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_last_assistant_text() {
        let messages = vec![
            Message::user("Hello"),
            Message::assistant("First response"),
            Message::user("Another question"),
            Message::assistant("Final answer"),
        ];
        assert_eq!(extract_last_assistant_text(&messages), "Final answer");
    }

    #[test]
    fn test_extract_last_assistant_text_no_assistant_returns_fallback() {
        let messages = vec![Message::user("Hello")];
        assert_eq!(
            extract_last_assistant_text(&messages),
            "(sub-agent produced no text output)"
        );
    }

    #[test]
    fn test_extract_last_assistant_text_empty_returns_fallback() {
        let messages: Vec<Message> = vec![];
        assert_eq!(
            extract_last_assistant_text(&messages),
            "(sub-agent produced no text output)"
        );
    }
}
