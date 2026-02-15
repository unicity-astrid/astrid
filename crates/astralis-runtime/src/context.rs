//! Context management and auto-summarization.
//!
//! Handles context window overflow by summarizing old messages.

use astralis_llm::{LlmProvider, Message, MessageContent};
use tracing::{debug, info};

use crate::error::RuntimeResult;
use crate::session::AgentSession;

/// Context manager for handling context overflow.
pub struct ContextManager {
    /// Maximum context tokens before summarization.
    max_context_tokens: usize,
    /// Threshold (0.0-1.0) at which to trigger summarization.
    summarization_threshold: f32,
    /// Number of recent messages to always keep.
    keep_recent_count: usize,
}

impl ContextManager {
    /// Create a new context manager.
    #[must_use]
    pub fn new(max_context_tokens: usize) -> Self {
        Self {
            max_context_tokens,
            summarization_threshold: 0.85,
            keep_recent_count: 10,
        }
    }

    /// Set the summarization threshold.
    #[must_use]
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.summarization_threshold = threshold.clamp(0.5, 0.95);
        self
    }

    /// Set how many recent messages to keep.
    #[must_use]
    pub fn keep_recent(mut self, count: usize) -> Self {
        self.keep_recent_count = count;
        self
    }

    /// Check if summarization is needed.
    #[must_use]
    pub fn needs_summarization(&self, session: &AgentSession) -> bool {
        session.is_near_limit(self.max_context_tokens, self.summarization_threshold)
    }

    /// Summarize old messages in a session.
    ///
    /// This removes old messages and replaces them with a summary.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM provider fails to generate a summary.
    pub async fn summarize<P: LlmProvider>(
        &self,
        session: &mut AgentSession,
        provider: &P,
    ) -> RuntimeResult<SummarizationResult> {
        if session.messages.len() <= self.keep_recent_count {
            return Ok(SummarizationResult {
                messages_evicted: 0,
                tokens_freed: 0,
                summary: None,
            });
        }

        // Safety: checked `len() > keep_recent_count` above
        #[allow(clippy::arithmetic_side_effects)]
        let evict_count = session.messages.len() - self.keep_recent_count;
        let messages_to_summarize: Vec<_> = session.messages.drain(..evict_count).collect();

        info!(
            evict_count = evict_count,
            remaining = session.messages.len(),
            "Summarizing old context"
        );

        // Calculate tokens freed (approximate)
        let tokens_freed: usize = messages_to_summarize
            .iter()
            .map(|m| match &m.content {
                MessageContent::Text(t) => t.len() / 4,
                _ => 100,
            })
            .sum();

        // Build summary prompt
        let messages_text = format_messages_for_summary(&messages_to_summarize);
        let summary_prompt = format!(
            "Summarize the following conversation, preserving key facts, decisions, \
             and context that would be important for continuing the conversation:\n\n{messages_text}"
        );

        // Get summary from LLM
        let summary = provider.complete_simple(&summary_prompt).await?;

        debug!(summary_len = summary.len(), "Generated context summary");

        // Insert summary as a system message at the beginning
        let summary_message =
            Message::system(format!("[Previous conversation summary]\n{summary}"));
        session.messages.insert(0, summary_message);

        // Update token count
        session.token_count = session.token_count.saturating_sub(tokens_freed);
        session.token_count = session.token_count.saturating_add(summary.len() / 4); // Add summary tokens

        Ok(SummarizationResult {
            messages_evicted: evict_count,
            tokens_freed,
            summary: Some(summary),
        })
    }

    /// Get context statistics.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn stats(&self, session: &AgentSession) -> ContextStats {
        let utilization = session.token_count as f32 / self.max_context_tokens as f32;

        ContextStats {
            current_tokens: session.token_count,
            max_tokens: self.max_context_tokens,
            utilization,
            message_count: session.messages.len(),
            needs_summarization: self.needs_summarization(session),
        }
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new(100_000) // Default to ~100k tokens
    }
}

/// Result of a summarization operation.
#[derive(Debug, Clone)]
pub struct SummarizationResult {
    /// Number of messages evicted.
    pub messages_evicted: usize,
    /// Approximate tokens freed.
    pub tokens_freed: usize,
    /// The generated summary (if any).
    pub summary: Option<String>,
}

/// Context statistics.
#[derive(Debug, Clone)]
pub struct ContextStats {
    /// Current token count.
    pub current_tokens: usize,
    /// Maximum allowed tokens.
    pub max_tokens: usize,
    /// Context utilization (0.0-1.0).
    pub utilization: f32,
    /// Number of messages.
    pub message_count: usize,
    /// Whether summarization is needed.
    pub needs_summarization: bool,
}

impl ContextStats {
    /// Get utilization as a percentage.
    #[must_use]
    pub fn utilization_percent(&self) -> f32 {
        self.utilization * 100.0
    }
}

/// Format messages for summarization.
fn format_messages_for_summary(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                astralis_llm::MessageRole::User => "User",
                astralis_llm::MessageRole::Assistant => "Assistant",
                astralis_llm::MessageRole::System => "System",
                astralis_llm::MessageRole::Tool => "Tool",
            };

            let content = match &m.content {
                MessageContent::Text(t) => t.clone(),
                MessageContent::ToolCalls(calls) => {
                    let call_strs: Vec<_> = calls
                        .iter()
                        .map(|c| format!("{}({})", &c.name, &c.arguments))
                        .collect();
                    let joined = call_strs.join(", ");
                    format!("[Tool calls: {joined}]")
                },
                MessageContent::ToolResult(r) => {
                    let result_content = if r.content.len() > 200 {
                        format!("{}...", &r.content[..200])
                    } else {
                        r.content.clone()
                    };
                    format!("[Tool result: {result_content}]")
                },
                MessageContent::MultiPart(_) => "[Multi-part content]".to_string(),
            };

            format!("{role}: {content}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_manager() {
        let manager = ContextManager::new(1000);
        let mut session = AgentSession::new([0u8; 8], "");

        // Add messages to exceed threshold
        for i in 0..50 {
            session.add_message(Message::user(format!("Message {i}")));
        }

        // Manually set high token count
        session.token_count = 900;

        assert!(manager.needs_summarization(&session));
    }

    #[test]
    fn test_context_stats() {
        let manager = ContextManager::new(1000);
        let mut session = AgentSession::new([0u8; 8], "");
        session.token_count = 500;

        let stats = manager.stats(&session);
        assert_eq!(stats.utilization, 0.5);
        assert_eq!(stats.utilization_percent(), 50.0);
    }
}
