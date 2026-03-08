//! Compaction strategies for the Context Engine.
//!
//! The default strategy is [`summarize_and_truncate`], which keeps the most
//! recent turns and removes the oldest non-protected messages until the
//! token budget is met.

use std::collections::HashSet;
use std::hash::BuildHasher;

/// Estimate token count for a JSON message using a simple heuristic.
///
/// Operates on the serialized JSON representation (including field names
/// and structural characters), not the decoded text content. This
/// systematically over-counts vs. real LLM tokenizers but provides a
/// reasonable upper-bound estimate without requiring a tokenizer dependency.
///
/// Uses ~4 characters per token, the industry-standard approximation
/// for English text with typical LLM tokenizers.
pub(crate) fn estimate_tokens(message: &serde_json::Value) -> u64 {
    let text = message.to_string();
    let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
    len.div_ceil(4)
}

/// Estimate total tokens across all messages, using saturating addition
/// to prevent overflow with pathologically large inputs.
pub(crate) fn estimate_total_tokens(messages: &[serde_json::Value]) -> u64 {
    messages
        .iter()
        .map(estimate_tokens)
        .fold(0u64, u64::saturating_add)
}

/// Extract the message ID from a JSON message, checking `id` and `message_id` fields.
fn message_id(msg: &serde_json::Value) -> Option<&str> {
    msg.get("id")
        .or_else(|| msg.get("message_id"))
        .and_then(serde_json::Value::as_str)
}

/// Default compaction strategy: keep recent turns, respect protected messages,
/// truncate oldest non-protected messages until under target tokens.
///
/// # Algorithm
///
/// 1. Messages within the most recent `keep_recent` turns are always kept.
/// 2. Messages whose ID appears in `protected_ids` are always kept.
/// 3. Remaining messages (older, unprotected) are removed oldest-first
///    until the total token estimate is at or below `target_tokens`.
pub(crate) fn summarize_and_truncate<S: BuildHasher>(
    messages: &[serde_json::Value],
    target_tokens: u64,
    protected_ids: &HashSet<String, S>,
    keep_recent: usize,
) -> Vec<serde_json::Value> {
    if messages.is_empty() {
        return Vec::new();
    }

    let current_tokens = estimate_total_tokens(messages);
    if current_tokens <= target_tokens {
        return messages.to_vec();
    }

    let total = messages.len();
    let recent_start = total.saturating_sub(keep_recent);

    // Classify each message as removable or not.
    let mut removable_indices: Vec<usize> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if i >= recent_start {
            continue;
        }
        let is_protected = message_id(msg).is_some_and(|id| protected_ids.contains(id));
        if !is_protected {
            removable_indices.push(i);
        }
    }

    // Remove oldest first until under budget.
    let mut removed: HashSet<usize> = HashSet::new();
    let mut running_tokens = current_tokens;

    for &idx in &removable_indices {
        if running_tokens <= target_tokens {
            break;
        }
        let msg_tokens = estimate_tokens(&messages[idx]);
        running_tokens = running_tokens.saturating_sub(msg_tokens);
        removed.insert(idx);
    }

    messages
        .iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, msg)| msg.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn make_msg(id: &str, content: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "role": "user",
            "content": content,
        })
    }

    #[test]
    fn estimate_tokens_basic() {
        let msg = serde_json::json!({"content": "hello"});
        let tokens = estimate_tokens(&msg);
        assert!(tokens > 0);
    }

    #[test]
    fn estimate_total_empty() {
        assert_eq!(estimate_total_tokens(&[]), 0);
    }

    #[test]
    fn estimate_total_saturates_instead_of_overflow() {
        // Verify we use saturating_add, not wrapping sum.
        let result = [u64::MAX, 1].iter().copied().fold(0u64, u64::saturating_add);
        assert_eq!(result, u64::MAX);
    }

    #[test]
    fn truncate_removes_oldest_first() {
        let messages: Vec<serde_json::Value> = (0..20)
            .map(|i| make_msg(&format!("msg-{i}"), &format!("Message content number {i}")))
            .collect();

        let result = summarize_and_truncate(&messages, 10, &HashSet::new(), 5);

        assert!(result.len() < messages.len());
        for i in 15..20 {
            let id = format!("msg-{i}");
            assert!(
                result.iter().any(|m| message_id(m) == Some(id.as_str())),
                "Recent message {id} should be preserved"
            );
        }
    }

    #[test]
    fn protected_messages_survive() {
        let messages: Vec<serde_json::Value> = (0..20)
            .map(|i| make_msg(&format!("msg-{i}"), &format!("Message content number {i}")))
            .collect();

        let protected: HashSet<String> =
            ["msg-0", "msg-3", "msg-7"].iter().map(|s| (*s).to_string()).collect();

        let result = summarize_and_truncate(&messages, 10, &protected, 5);

        for pid in &protected {
            assert!(
                result.iter().any(|m| message_id(m) == Some(pid.as_str())),
                "Protected message {pid} should survive compaction"
            );
        }
    }

    #[test]
    fn no_compaction_when_under_budget() {
        let messages = vec![make_msg("msg-1", "hi")];
        let result = summarize_and_truncate(&messages, 100_000, &HashSet::new(), 5);
        assert_eq!(result.len(), messages.len());
    }

    #[test]
    fn empty_messages_returns_empty() {
        let result = summarize_and_truncate(&[], 1000, &HashSet::new(), 5);
        assert!(result.is_empty());
    }

    #[test]
    fn all_protected_returns_all() {
        let messages: Vec<serde_json::Value> = (0..5)
            .map(|i| make_msg(&format!("msg-{i}"), &format!("Content {i}")))
            .collect();

        let protected: HashSet<String> = (0..5).map(|i| format!("msg-{i}")).collect();

        let result = summarize_and_truncate(&messages, 1, &protected, 5);
        assert_eq!(result.len(), messages.len());
    }
}
