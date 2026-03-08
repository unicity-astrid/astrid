#![deny(unsafe_code)]
#![deny(clippy::all)]

//! Context Engine capsule — pluggable compaction with interceptor hook support.
//!
//! Manages context window compaction and fires `before_compaction` /
//! `after_compaction` interceptors to plugin capsules via IPC. The default
//! strategy is simple token-budget trimming. A different capsule claiming
//! the same IPC topics can replace this one entirely.
//!
//! # IPC Protocol
//!
//! **Requests** (publish to these topics):
//! - `context_engine.compact` — compact a session's messages
//! - `context_engine.estimate_tokens` — estimate token count for messages
//!
//! **Responses** (published by context engine):
//! - `context_engine.response.compact` — compacted messages and stats
//! - `context_engine.response.estimate_tokens` — estimated token count
//!
//! # Interceptor Hooks (fired via IPC)
//!
//! - `before_compaction` — request-response: plugins can mark messages as
//!   protected or skip compaction entirely. Plugins respond on a per-request
//!   response topic included in the hook payload.
//! - `after_compaction` — fire-and-forget notification with stats.

mod strategy;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use astrid_sdk::prelude::*;
use extism_pdk::FnResult;
use serde::{Deserialize, Serialize};

// ── IPC payload types ───────────────────────────────────────────────

/// IPC request payload for `context_engine.compact`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactRequest {
    /// Session being compacted.
    session_id: String,
    /// Current conversation messages.
    messages: Vec<serde_json::Value>,
    /// Hard limit on context window size (tokens).
    max_tokens: u64,
    /// Target token count after compaction.
    target_tokens: u64,
}

/// IPC response payload for `context_engine.response.compact`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactResponse {
    /// Messages after compaction.
    messages: Vec<serde_json::Value>,
    /// Whether compaction actually occurred.
    compacted: bool,
    /// Number of messages removed.
    messages_removed: u32,
    /// Strategy that was used.
    strategy: String,
}

/// IPC request payload for `context_engine.estimate_tokens`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EstimateRequest {
    /// Messages to estimate token count for.
    messages: Vec<serde_json::Value>,
}

/// IPC response payload for `context_engine.response.estimate_tokens`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EstimateResponse {
    /// Estimated total token count.
    estimated_tokens: u64,
}

/// Payload sent to plugin capsules via the `before_compaction` IPC hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BeforeCompactionPayload {
    /// Session being compacted.
    session_id: String,
    /// Current messages.
    messages: Vec<serde_json::Value>,
    /// Number of messages.
    message_count: u32,
    /// Estimated current token usage.
    estimated_tokens: u64,
    /// Maximum allowed tokens.
    max_tokens: u64,
    /// Topic where plugins should publish their hook responses.
    response_topic: String,
}

/// A single plugin's response to the `before_compaction` hook.
///
/// All fields are optional. The context engine merges responses from
/// multiple plugins: any `skip: true` wins, `protected_message_ids`
/// are unioned.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeforeCompactionHookResponse {
    /// If `true`, skip compaction entirely (plugin takes responsibility).
    #[serde(default)]
    skip: Option<bool>,
    /// Message IDs that must not be removed or summarized.
    #[serde(default, alias = "protected_message_ids")]
    pinned_message_ids: Vec<String>,
    /// Reserved for future use: plugin-provided compaction strategy name.
    #[serde(default)]
    custom_strategy: Option<String>,
}

impl BeforeCompactionHookResponse {
    /// Returns `true` if at least one field is set.
    fn has_any_field(&self) -> bool {
        self.skip.is_some() || !self.pinned_message_ids.is_empty() || self.custom_strategy.is_some()
    }
}

/// Fire-and-forget notification payload for `after_compaction`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AfterCompactionPayload {
    /// Session that was compacted.
    session_id: String,
    /// Message count before compaction.
    messages_before: u32,
    /// Message count after compaction.
    messages_after: u32,
    /// Token estimate before compaction.
    tokens_before: u64,
    /// Token estimate after compaction.
    tokens_after: u64,
    /// Name of the strategy used.
    strategy_used: String,
}

// ── Configuration ───────────────────────────────────────────────────

/// Runtime configuration loaded from capsule config at startup.
struct Config {
    /// Maximum time (in milliseconds) to wait for plugin hook responses.
    hook_timeout_ms: u64,
    /// Number of recent turns to always keep during compaction.
    keep_recent: usize,
}

impl Config {
    fn load() -> Self {
        let hook_timeout_ms = sys::get_config_string("hook_timeout_ms")
            .ok()
            .and_then(|s| s.trim().trim_matches('"').parse::<u64>().ok())
            .unwrap_or(DEFAULT_HOOK_POLL_TIMEOUT_MS);

        let keep_recent = sys::get_config_string("keep_recent")
            .ok()
            .and_then(|s| s.trim().trim_matches('"').parse::<usize>().ok())
            .unwrap_or(DEFAULT_KEEP_RECENT);

        Self {
            hook_timeout_ms,
            keep_recent,
        }
    }
}

// ── Constants ───────────────────────────────────────────────────────

/// Default maximum time to wait for plugin hook responses.
const DEFAULT_HOOK_POLL_TIMEOUT_MS: u64 = 2000;

/// Poll interval between checking for hook responses.
const HOOK_POLL_INTERVAL_MS: u64 = 10;

/// Maximum number of hook responses to collect.
const MAX_HOOK_RESPONSES: usize = 50;

/// Default number of recent turns to always keep during compaction.
const DEFAULT_KEEP_RECENT: usize = 10;

/// Monotonic counter for unique request IDs.
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

// ── Main entry point ────────────────────────────────────────────────

#[plugin_fn]
pub fn run() -> FnResult<()> {
    let _ = sys::log("info", "Context Engine capsule starting");

    let config = Config::load();
    let _ = sys::log(
        "info",
        format!(
            "Hook timeout: {}ms, keep_recent: {}",
            config.hook_timeout_ms, config.keep_recent
        ),
    );

    let sub = ipc::subscribe("context_engine.*")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // Subscribe to our own hook topics so we can drain them.
    let hook_sub = ipc::subscribe("before_compaction")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;
    let after_sub = ipc::subscribe("after_compaction")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    let _ = sys::log("info", "Context Engine capsule ready");

    loop {
        match ipc::poll_bytes(&sub) {
            Ok(bytes) => {
                if !bytes.is_empty() {
                    handle_poll_envelope(&bytes, &config);
                }
            },
            Err(_) => break,
        }

        // Drain hook topics to prevent backpressure.
        let _ = ipc::poll_bytes(&hook_sub);
        let _ = ipc::poll_bytes(&after_sub);
    }

    let _ = ipc::unsubscribe(&sub);
    let _ = ipc::unsubscribe(&hook_sub);
    let _ = ipc::unsubscribe(&after_sub);

    Ok(())
}

// ── Envelope dispatch ───────────────────────────────────────────────

/// Returns `true` if the topic should be dispatched (not a self-echo).
fn should_dispatch_topic(topic: &str) -> bool {
    !topic.starts_with("context_engine.response.")
        && !topic.starts_with("context_engine.hook_response.")
        && topic != "before_compaction"
        && topic != "after_compaction"
}

/// Parse the poll envelope and dispatch individual messages.
fn handle_poll_envelope(poll_bytes: &[u8], config: &Config) {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => return,
    };

    if let Some(dropped) = envelope.get("dropped").and_then(|d| d.as_u64())
        && dropped > 0
    {
        let _ = sys::log(
            "warn",
            format!("Event bus dropped {dropped} messages in context engine poll"),
        );
    }

    let messages = match envelope.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for msg in messages {
        let topic = match msg.get("topic").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        if !should_dispatch_topic(topic) {
            continue;
        }

        let payload = match msg.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // Extract from Custom payload envelope or direct.
        let request_value = payload.get("data").unwrap_or(payload);

        match topic {
            "context_engine.compact" => handle_compact(request_value, config),
            "context_engine.estimate_tokens" => handle_estimate_tokens(request_value),
            _ => {},
        }
    }
}

// ── Compact handler ─────────────────────────────────────────────────

/// Handle a `context_engine.compact` request.
///
/// 1. Parse the request
/// 2. Clamp `target_tokens` to not exceed `max_tokens`
/// 3. Fire `before_compaction` hook via IPC
/// 4. Merge responses (any skip → skip, union of pinned IDs)
/// 5. Run compaction strategy
/// 6. Fire `after_compaction` notification
/// 7. Publish compacted result
fn handle_compact(payload: &serde_json::Value, config: &Config) {
    let request: CompactRequest = match serde_json::from_value(payload.clone()) {
        Ok(r) => r,
        Err(e) => {
            let _ = sys::log("error", format!("Failed to parse compact request: {e}"));
            let _ = ipc::publish_json(
                "context_engine.response.compact",
                &serde_json::json!({"error": format!("invalid request: {e}")}),
            );
            return;
        },
    };

    // Enforce: target_tokens must not exceed max_tokens.
    let target_tokens = request.target_tokens.min(request.max_tokens);

    let message_count = u32::try_from(request.messages.len()).unwrap_or(u32::MAX);
    let tokens_before = strategy::estimate_total_tokens(&request.messages);

    // Fire `before_compaction` hook via IPC.
    let merged = fire_before_compaction(&request, tokens_before, message_count, config);

    // If any plugin says skip, return messages unchanged.
    if merged.skip {
        let _ = sys::log(
            "info",
            format!("Compaction skipped by plugin for session {}", request.session_id),
        );
        let response = CompactResponse {
            messages: request.messages,
            compacted: false,
            messages_removed: 0,
            strategy: "skipped".to_string(),
        };
        let _ = ipc::publish_json("context_engine.response.compact", &response);
        return;
    }

    // Run default compaction strategy.
    let compacted_messages = strategy::summarize_and_truncate(
        &request.messages,
        target_tokens,
        &merged.protected_ids,
        config.keep_recent,
    );

    let messages_after = u32::try_from(compacted_messages.len()).unwrap_or(u32::MAX);
    let messages_removed = message_count.saturating_sub(messages_after);
    let tokens_after = strategy::estimate_total_tokens(&compacted_messages);
    let compacted = messages_removed > 0;
    let strategy_name = "summarize_and_truncate".to_string();

    // Fire `after_compaction` notification (fire-and-forget).
    fire_after_compaction(
        &request.session_id,
        message_count,
        messages_after,
        tokens_before,
        tokens_after,
        &strategy_name,
    );

    // Publish the compacted result.
    let response = CompactResponse {
        messages: compacted_messages,
        compacted,
        messages_removed,
        strategy: strategy_name,
    };
    let _ = ipc::publish_json("context_engine.response.compact", &response);

    let _ = sys::log(
        "info",
        format!(
            "Compaction completed: session={}, removed={messages_removed}, \
             tokens {tokens_before} → {tokens_after}",
            request.session_id
        ),
    );
}

// ── Estimate handler ────────────────────────────────────────────────

/// Handle a `context_engine.estimate_tokens` request.
fn handle_estimate_tokens(payload: &serde_json::Value) {
    let request: EstimateRequest = match serde_json::from_value(payload.clone()) {
        Ok(r) => r,
        Err(e) => {
            let _ = sys::log("error", format!("Failed to parse estimate_tokens request: {e}"));
            let _ = ipc::publish_json(
                "context_engine.response.estimate_tokens",
                &serde_json::json!({"error": format!("invalid request: {e}")}),
            );
            return;
        },
    };

    let estimated_tokens = strategy::estimate_total_tokens(&request.messages);
    let response = EstimateResponse { estimated_tokens };
    let _ = ipc::publish_json("context_engine.response.estimate_tokens", &response);
}

// ── Interceptor hook firing via IPC ─────────────────────────────────

/// Merged result of all `before_compaction` hook responses.
struct MergedBeforeCompaction {
    /// If `true`, skip compaction entirely.
    skip: bool,
    /// Union of all pinned/protected message IDs.
    protected_ids: HashSet<String>,
}

/// Fire the `before_compaction` hook via IPC and collect plugin responses.
///
/// Publishes the hook payload on the `before_compaction` IPC topic with a
/// per-request response topic. Polls for plugin responses within the
/// configured timeout window.
fn fire_before_compaction(
    request: &CompactRequest,
    tokens_before: u64,
    message_count: u32,
    config: &Config,
) -> MergedBeforeCompaction {
    let request_id = format!(
        "compact-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );

    let response_topic = format!("context_engine.hook_response.{request_id}");

    // Subscribe BEFORE publishing to avoid missing fast responses.
    let sub = match ipc::subscribe(&response_topic) {
        Ok(h) => h,
        Err(e) => {
            let _ = sys::log(
                "error",
                format!("Failed to subscribe to hook response topic: {e}"),
            );
            return MergedBeforeCompaction {
                skip: false,
                protected_ids: HashSet::new(),
            };
        },
    };

    let payload = BeforeCompactionPayload {
        session_id: request.session_id.clone(),
        messages: request.messages.clone(),
        message_count,
        estimated_tokens: tokens_before,
        max_tokens: request.max_tokens,
        response_topic: response_topic.clone(),
    };

    if let Err(e) = ipc::publish_json("before_compaction", &payload) {
        let _ = sys::log(
            "error",
            format!("Failed to publish before_compaction event: {e}"),
        );
        let _ = ipc::unsubscribe(&sub);
        return MergedBeforeCompaction {
            skip: false,
            protected_ids: HashSet::new(),
        };
    }

    // Poll for responses with configurable timeout.
    let mut responses: Vec<BeforeCompactionHookResponse> = Vec::new();
    let max_iterations = (config.hook_timeout_ms / HOOK_POLL_INTERVAL_MS).max(1);

    for _ in 0..max_iterations {
        if let Ok(bytes) = ipc::poll_bytes(&sub)
            && !bytes.is_empty()
            && let Some(new_responses) = parse_hook_responses(&bytes)
        {
            responses.extend(new_responses);
            if responses.len() >= MAX_HOOK_RESPONSES {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(HOOK_POLL_INTERVAL_MS));
    }

    let _ = ipc::unsubscribe(&sub);

    if !responses.is_empty() {
        let _ = sys::log(
            "info",
            format!("Collected {} before_compaction responses", responses.len()),
        );
    }

    merge_before_compaction_responses(&responses)
}

/// Parse the IPC poll envelope and extract hook responses.
fn parse_hook_responses(poll_bytes: &[u8]) -> Option<Vec<BeforeCompactionHookResponse>> {
    let envelope: serde_json::Value = serde_json::from_slice(poll_bytes).ok()?;
    let messages = envelope.get("messages")?.as_array()?;
    let mut responses = Vec::new();

    for msg in messages {
        let payload = match msg.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // Try direct payload, then nested in Custom `data` envelope.
        let maybe_response = serde_json::from_value::<BeforeCompactionHookResponse>(payload.clone())
            .ok()
            .filter(BeforeCompactionHookResponse::has_any_field)
            .or_else(|| {
                payload
                    .get("data")
                    .and_then(|data| {
                        serde_json::from_value::<BeforeCompactionHookResponse>(data.clone()).ok()
                    })
                    .filter(BeforeCompactionHookResponse::has_any_field)
            });

        if let Some(response) = maybe_response {
            responses.push(response);
        }
    }

    if responses.is_empty() {
        None
    } else {
        Some(responses)
    }
}

/// Merge `before_compaction` responses from multiple plugins.
///
/// - `skip`: any `true` → skip compaction
/// - `pinned_message_ids`: union of all responses
fn merge_before_compaction_responses(
    responses: &[BeforeCompactionHookResponse],
) -> MergedBeforeCompaction {
    let skip = responses
        .iter()
        .any(|r| r.skip == Some(true));

    let protected_ids: HashSet<String> = responses
        .iter()
        .flat_map(|r| r.pinned_message_ids.iter().cloned())
        .collect();

    MergedBeforeCompaction { skip, protected_ids }
}

/// Fire the `after_compaction` notification (fire-and-forget).
fn fire_after_compaction(
    session_id: &str,
    messages_before: u32,
    messages_after: u32,
    tokens_before: u64,
    tokens_after: u64,
    strategy_used: &str,
) {
    let payload = AfterCompactionPayload {
        session_id: session_id.to_string(),
        messages_before,
        messages_after,
        tokens_before,
        tokens_after,
        strategy_used: strategy_used.to_string(),
    };
    let _ = ipc::publish_json("after_compaction", &payload);
}

#[cfg(test)]
mod tests;
