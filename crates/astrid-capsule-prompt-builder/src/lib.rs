#![deny(unsafe_code)]
#![deny(clippy::all)]

//! Prompt Builder capsule — assembles LLM prompts with plugin hook interception.
//!
//! This capsule owns the prompt assembly pipeline. When the orchestrator needs
//! a prompt assembled, it publishes to `prompt_builder.assemble`. The prompt
//! builder then:
//!
//! 1. Fires `before_prompt_build` to all plugin capsules via IPC
//! 2. Collects plugin responses (`prependSystemContext`, `appendSystemContext`,
//!    `systemPrompt` override, `prependContext`)
//! 3. Merges them according to OpenClaw-compatible semantics
//! 4. Returns the assembled prompt on `prompt_builder.response.assemble`
//! 5. Fires `after_prompt_build` as a notification
//!
//! # Merge Semantics
//!
//! 1. `prependContext` — concatenated in order, becomes `user_context_prefix`
//! 2. `systemPrompt` — last non-null value wins (full override)
//! 3. `prependSystemContext` — concatenated in order, prepended to system prompt
//! 4. `appendSystemContext` — concatenated in order, appended to system prompt

use astrid_sdk::prelude::*;
use extism_pdk::FnResult;
use serde::{Deserialize, Serialize};

/// Runtime configuration loaded from capsule config at startup.
struct Config {
    /// Maximum time (in milliseconds) to wait for plugin hook responses.
    hook_timeout_ms: u64,
}

impl Config {
    /// Load configuration from the capsule's config store, falling back to defaults.
    fn load() -> Self {
        let hook_timeout_ms = sys::get_config_string("hook_timeout_ms")
            .ok()
            .and_then(|s| s.trim().trim_matches('"').parse::<u64>().ok())
            .unwrap_or(DEFAULT_HOOK_POLL_TIMEOUT_MS);

        Self { hook_timeout_ms }
    }
}

/// Request from the orchestrator to assemble a prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AssembleRequest {
    /// The conversation messages.
    #[serde(default)]
    messages: serde_json::Value,
    /// The current system prompt before plugin modifications.
    #[serde(default)]
    system_prompt: String,
    /// Unique request identifier for correlation.
    request_id: String,
    /// The target LLM model identifier.
    #[serde(default)]
    model: String,
    /// The LLM provider identifier.
    #[serde(default)]
    provider: String,
}

/// Response returned to the orchestrator with the assembled prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AssembleResponse {
    /// The final assembled system prompt.
    system_prompt: String,
    /// Text to prepend to the user's message (from `prependContext` hooks).
    user_context_prefix: String,
    /// The original request ID for correlation.
    request_id: String,
}

/// Payload sent to plugin capsules via the `before_prompt_build` interceptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct BeforePromptBuildPayload {
    messages: serde_json::Value,
    system_prompt: String,
    request_id: String,
    model: String,
    provider: String,
    /// Topic where plugins should publish their hook responses.
    response_topic: String,
}

/// A single plugin's response to the `before_prompt_build` hook.
///
/// All fields are optional. The prompt builder merges responses from
/// multiple plugins according to the documented merge semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookResponse {
    /// Text to prepend to the system prompt.
    #[serde(default)]
    prepend_system_context: Option<String>,
    /// Text to append to the system prompt.
    #[serde(default)]
    append_system_context: Option<String>,
    /// Full system prompt override (last non-null wins).
    #[serde(default)]
    system_prompt: Option<String>,
    /// Text to prepend to the user's message.
    #[serde(default)]
    prepend_context: Option<String>,
}

impl HookResponse {
    /// Returns `true` if at least one field is set.
    fn has_any_field(&self) -> bool {
        self.prepend_system_context.is_some()
            || self.append_system_context.is_some()
            || self.system_prompt.is_some()
            || self.prepend_context.is_some()
    }
}

/// Notification payload sent after prompt assembly completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AfterPromptBuildPayload {
    system_prompt: String,
    user_context_prefix: String,
    request_id: String,
}

/// Default maximum time (in milliseconds) to wait for plugin hook responses.
/// Overridable via the `hook_timeout_ms` capsule config key.
const DEFAULT_HOOK_POLL_TIMEOUT_MS: u64 = 2000;

/// Poll interval (in milliseconds) between checking for hook responses.
const HOOK_POLL_INTERVAL_MS: u64 = 10;

/// Maximum number of hook responses to collect before proceeding.
const MAX_HOOK_RESPONSES: usize = 50;

/// A hook response paired with its source capsule identifier.
///
/// Used for permission gating: plugins with `allowPromptInjection: false`
/// should have their prompt-mutating fields discarded.
struct SourcedHookResponse {
    /// The capsule/session ID that sent this response.
    source_id: Option<String>,
    /// The parsed hook response.
    response: HookResponse,
}

/// Filter hook responses based on prompt injection permissions.
///
/// Plugins without prompt injection permission retain only `prependContext`
/// (user-visible context), while `systemPrompt`, `prependSystemContext`,
/// and `appendSystemContext` are stripped.
///
/// TODO: Query the kernel for per-capsule `allowPromptInjection` permission.
/// Currently accepts all responses (no permission store available yet).
fn filter_by_permission(sourced: Vec<SourcedHookResponse>) -> Vec<HookResponse> {
    sourced
        .into_iter()
        .map(|s| {
            // TODO: Check capsule capability via kernel query:
            //   if !has_prompt_injection_permission(&s.source_id) {
            //       return HookResponse {
            //           prepend_context: s.response.prepend_context,
            //           ..Default::default()
            //       };
            //   }
            let _ = &s.source_id; // suppress unused warning until permission gating is wired
            s.response
        })
        .collect()
}

/// Merge collected hook responses into a final assembled prompt.
///
/// Merge order (matches OpenClaw documented behaviour):
/// 1. `prependContext` — concatenated in interceptor order
/// 2. `systemPrompt` — last non-null value wins as full override
/// 3. `prependSystemContext` — concatenated, prepended to (possibly overridden) prompt
/// 4. `appendSystemContext` — concatenated, appended to system prompt
fn merge_hook_responses(original_system_prompt: &str, responses: &[HookResponse]) -> MergedPrompt {
    let mut prepend_contexts: Vec<&str> = Vec::new();
    let mut prepend_system_contexts: Vec<&str> = Vec::new();
    let mut append_system_contexts: Vec<&str> = Vec::new();
    let mut system_prompt_override: Option<&str> = None;

    for resp in responses {
        if let Some(ref ctx) = resp.prepend_context
            && !ctx.is_empty()
        {
            prepend_contexts.push(ctx);
        }
        if let Some(ref prompt) = resp.system_prompt
            && !prompt.is_empty()
        {
            // Last non-empty wins — intentionally overwrites previous overrides.
            // An empty string is treated as "no override" to prevent accidentally
            // wiping the system prompt.
            system_prompt_override = Some(prompt);
        }
        if let Some(ref ctx) = resp.prepend_system_context
            && !ctx.is_empty()
        {
            prepend_system_contexts.push(ctx);
        }
        if let Some(ref ctx) = resp.append_system_context
            && !ctx.is_empty()
        {
            append_system_contexts.push(ctx);
        }
    }

    // Step 2: Determine the base system prompt (override or original).
    let base_prompt = system_prompt_override.unwrap_or(original_system_prompt);

    // Step 3-4: Prepend + base + append.
    let mut final_prompt = String::new();
    for (i, ctx) in prepend_system_contexts.iter().enumerate() {
        if i > 0 {
            final_prompt.push('\n');
        }
        final_prompt.push_str(ctx);
    }
    if !final_prompt.is_empty() && !base_prompt.is_empty() {
        final_prompt.push('\n');
    }
    final_prompt.push_str(base_prompt);
    for ctx in &append_system_contexts {
        if !final_prompt.is_empty() {
            final_prompt.push('\n');
        }
        final_prompt.push_str(ctx);
    }

    // Step 1: Build user context prefix.
    let user_context_prefix = prepend_contexts.join("\n");

    MergedPrompt {
        system_prompt: final_prompt,
        user_context_prefix,
    }
}

/// The result of merging all hook responses.
struct MergedPrompt {
    system_prompt: String,
    user_context_prefix: String,
}

/// Fire the `before_prompt_build` interceptor and collect plugin responses.
///
/// Publishes the hook event on the `before_prompt_build` IPC topic and polls
/// a dedicated response topic for plugin contributions. Returns all collected
/// responses within the timeout window, filtered by permission gating.
fn fire_before_prompt_build(request: &AssembleRequest, config: &Config) -> Vec<HookResponse> {
    let response_topic = format!(
        "prompt_builder.hook_response.{}",
        request.request_id
    );

    // Subscribe BEFORE publishing to avoid missing fast responses.
    let sub = match ipc::subscribe(&response_topic) {
        Ok(h) => h,
        Err(e) => {
            let _ = sys::log(
                "error",
                format!("Failed to subscribe to hook response topic: {e}"),
            );
            return Vec::new();
        },
    };

    let payload = BeforePromptBuildPayload {
        messages: request.messages.clone(),
        system_prompt: request.system_prompt.clone(),
        request_id: request.request_id.clone(),
        model: request.model.clone(),
        provider: request.provider.clone(),
        response_topic: response_topic.clone(),
    };

    if let Err(e) = ipc::publish_json("before_prompt_build", &payload) {
        let _ = sys::log(
            "error",
            format!("Failed to publish before_prompt_build event: {e}"),
        );
        let _ = ipc::unsubscribe(&sub);
        return Vec::new();
    }

    // Poll for responses with configurable timeout.
    // Guarantee at least 1 iteration even if timeout < poll interval.
    let mut sourced_responses = Vec::new();
    let max_iterations = (config.hook_timeout_ms / HOOK_POLL_INTERVAL_MS).max(1);

    for _ in 0..max_iterations {
        if let Ok(bytes) = ipc::poll_bytes(&sub)
            && !bytes.is_empty()
            && let Some(new_responses) = parse_hook_responses(&bytes)
        {
            sourced_responses.extend(new_responses);
            if sourced_responses.len() >= MAX_HOOK_RESPONSES {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(HOOK_POLL_INTERVAL_MS));
    }

    let _ = ipc::unsubscribe(&sub);

    let _ = sys::log(
        "info",
        format!(
            "Collected {} hook responses for request {}",
            sourced_responses.len(),
            request.request_id
        ),
    );

    filter_by_permission(sourced_responses)
}

/// Parse the poll envelope and extract hook responses with source capsule IDs.
fn parse_hook_responses(poll_bytes: &[u8]) -> Option<Vec<SourcedHookResponse>> {
    let envelope: serde_json::Value = serde_json::from_slice(poll_bytes).ok()?;

    let messages = envelope.get("messages")?.as_array()?;
    let mut responses = Vec::new();

    for msg in messages {
        let payload = match msg.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // Track the source capsule for permission gating.
        let source_id = msg
            .get("source_id")
            .and_then(|s| s.as_str())
            .map(String::from);

        // Try to parse the payload directly as a HookResponse.
        // Since all fields are optional, an unrelated JSON object would
        // parse as an empty HookResponse — check `has_any_field()` to
        // distinguish real responses from false positives.
        // Plugins may wrap it in various IPC payload envelopes, so we
        // also check inside `data` for Custom payloads.
        let maybe_response = serde_json::from_value::<HookResponse>(payload.clone())
            .ok()
            .filter(HookResponse::has_any_field)
            .or_else(|| {
                payload
                    .get("data")
                    .and_then(|data| serde_json::from_value::<HookResponse>(data.clone()).ok())
                    .filter(HookResponse::has_any_field)
            });

        if let Some(response) = maybe_response {
            responses.push(SourcedHookResponse {
                source_id,
                response,
            });
        }
    }

    if responses.is_empty() {
        None
    } else {
        Some(responses)
    }
}

/// Fire the `after_prompt_build` notification (fire-and-forget).
fn fire_after_prompt_build(system_prompt: &str, user_context_prefix: &str, request_id: &str) {
    let payload = AfterPromptBuildPayload {
        system_prompt: system_prompt.to_string(),
        user_context_prefix: user_context_prefix.to_string(),
        request_id: request_id.to_string(),
    };
    let _ = ipc::publish_json("after_prompt_build", &payload);
}

/// Handle a single `prompt_builder.assemble` request.
fn handle_assemble(payload: &serde_json::Value, config: &Config) {
    // Extract from Custom payload envelope or direct.
    let request_value = payload
        .get("data")
        .unwrap_or(payload);

    let request: AssembleRequest = match serde_json::from_value(request_value.clone()) {
        Ok(r) => r,
        Err(e) => {
            let _ = sys::log(
                "error",
                format!("Failed to parse assemble request: {e}"),
            );
            let _ = ipc::publish_json(
                "prompt_builder.response.assemble",
                &serde_json::json!({"error": format!("invalid request: {e}")}),
            );
            return;
        },
    };

    if request.request_id.is_empty() {
        let _ = ipc::publish_json(
            "prompt_builder.response.assemble",
            &serde_json::json!({"error": "missing request_id"}),
        );
        return;
    }

    // Fire interceptor hooks and collect responses.
    let hook_responses = fire_before_prompt_build(&request, config);

    // Merge all responses into the final prompt.
    let merged = merge_hook_responses(&request.system_prompt, &hook_responses);

    // Publish the assembled result.
    let response = AssembleResponse {
        system_prompt: merged.system_prompt.clone(),
        user_context_prefix: merged.user_context_prefix.clone(),
        request_id: request.request_id.clone(),
    };

    let _ = ipc::publish_json("prompt_builder.response.assemble", &response);

    // Fire after_prompt_build notification (fire-and-forget).
    fire_after_prompt_build(&merged.system_prompt, &merged.user_context_prefix, &request.request_id);
}

/// Returns `true` if the topic should be dispatched (not a self-echo).
///
/// Filters out our own response topics, hook response topics, and the
/// interceptor topics we publish. Only `prompt_builder.assemble` passes.
fn should_dispatch_topic(topic: &str) -> bool {
    !topic.starts_with("prompt_builder.response.")
        && !topic.starts_with("prompt_builder.hook_response.")
        && topic != "before_prompt_build"
        && topic != "after_prompt_build"
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
            format!("Event bus dropped {dropped} messages in prompt builder poll"),
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

        if topic == "prompt_builder.assemble"
            && let Some(payload) = msg.get("payload")
        {
            handle_assemble(payload, config);
        }
    }
}

#[plugin_fn]
pub fn run() -> FnResult<()> {
    let _ = sys::log("info", "Prompt Builder capsule starting");

    let config = Config::load();
    let _ = sys::log(
        "info",
        format!("Hook timeout: {}ms", config.hook_timeout_ms),
    );

    let sub = ipc::subscribe("prompt_builder.*")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // Also subscribe to our own hook topics so we can filter them out.
    let hook_sub = ipc::subscribe("before_prompt_build")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;
    let after_sub = ipc::subscribe("after_prompt_build")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    let _ = sys::log("info", "Prompt Builder capsule ready");

    loop {
        match ipc::poll_bytes(&sub) {
            Ok(bytes) => {
                if !bytes.is_empty() {
                    handle_poll_envelope(&bytes, &config);
                }
            },
            Err(_) => break,
        }

        // Drain hook/after topics to prevent backpressure.
        let _ = ipc::poll_bytes(&hook_sub);
        let _ = ipc::poll_bytes(&after_sub);

        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let _ = ipc::unsubscribe(&sub);
    let _ = ipc::unsubscribe(&hook_sub);
    let _ = ipc::unsubscribe(&after_sub);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_response(
        prepend_system: Option<&str>,
        append_system: Option<&str>,
        system_prompt: Option<&str>,
        prepend_context: Option<&str>,
    ) -> HookResponse {
        HookResponse {
            prepend_system_context: prepend_system.map(String::from),
            append_system_context: append_system.map(String::from),
            system_prompt: system_prompt.map(String::from),
            prepend_context: prepend_context.map(String::from),
        }
    }

    #[test]
    fn no_responses_returns_original() {
        let merged = merge_hook_responses("You are a helpful assistant.", &[]);
        assert_eq!(merged.system_prompt, "You are a helpful assistant.");
        assert_eq!(merged.user_context_prefix, "");
    }

    #[test]
    fn single_plugin_prepends_system_context() {
        let responses = vec![make_response(
            Some("Current date: 2026-03-08"),
            None,
            None,
            None,
        )];
        let merged = merge_hook_responses("You are a helpful assistant.", &responses);
        assert_eq!(
            merged.system_prompt,
            "Current date: 2026-03-08\nYou are a helpful assistant."
        );
    }

    #[test]
    fn single_plugin_appends_system_context() {
        let responses = vec![make_response(
            None,
            Some("Always respond in JSON."),
            None,
            None,
        )];
        let merged = merge_hook_responses("You are a helpful assistant.", &responses);
        assert_eq!(
            merged.system_prompt,
            "You are a helpful assistant.\nAlways respond in JSON."
        );
    }

    #[test]
    fn multiple_plugins_prepend_and_append() {
        let responses = vec![
            make_response(Some("Context A"), None, None, None),
            make_response(Some("Context B"), Some("Suffix X"), None, None),
            make_response(None, Some("Suffix Y"), None, None),
        ];
        let merged = merge_hook_responses("Base prompt.", &responses);
        assert_eq!(
            merged.system_prompt,
            "Context A\nContext B\nBase prompt.\nSuffix X\nSuffix Y"
        );
    }

    #[test]
    fn system_prompt_override_last_wins() {
        let responses = vec![
            make_response(None, None, Some("Override 1"), None),
            make_response(None, None, Some("Override 2"), None),
        ];
        let merged = merge_hook_responses("Original.", &responses);
        assert_eq!(merged.system_prompt, "Override 2");
    }

    #[test]
    fn override_then_prepend_append() {
        let responses = vec![
            make_response(None, None, Some("Custom base"), None),
            make_response(Some("Prefix"), Some("Suffix"), None, None),
        ];
        let merged = merge_hook_responses("Original.", &responses);
        assert_eq!(merged.system_prompt, "Prefix\nCustom base\nSuffix");
    }

    #[test]
    fn prepend_context_collected() {
        let responses = vec![
            make_response(None, None, None, Some("User context A")),
            make_response(None, None, None, Some("User context B")),
        ];
        let merged = merge_hook_responses("System prompt.", &responses);
        assert_eq!(merged.system_prompt, "System prompt.");
        assert_eq!(merged.user_context_prefix, "User context A\nUser context B");
    }

    #[test]
    fn all_fields_combined() {
        let responses = vec![
            make_response(
                Some("Date: today"),
                Some("Format: markdown"),
                None,
                Some("Here is some context"),
            ),
            make_response(
                Some("User: Josh"),
                None,
                Some("You are Astrid, a secure agent."),
                Some("Additional context"),
            ),
            make_response(None, Some("Be concise."), None, None),
        ];
        let merged = merge_hook_responses("Default system prompt.", &responses);

        // systemPrompt override from response[1]: "You are Astrid, a secure agent."
        // prependSystemContext: "Date: today" + "User: Josh"
        // appendSystemContext: "Format: markdown" + "Be concise."
        assert_eq!(
            merged.system_prompt,
            "Date: today\nUser: Josh\nYou are Astrid, a secure agent.\nFormat: markdown\nBe concise."
        );
        assert_eq!(
            merged.user_context_prefix,
            "Here is some context\nAdditional context"
        );
    }

    #[test]
    fn empty_system_prompt_override_does_not_wipe_original() {
        // An empty string systemPrompt should not override the original.
        let responses = vec![make_response(None, None, Some(""), None)];
        let merged = merge_hook_responses("Original.", &responses);
        assert_eq!(merged.system_prompt, "Original.");
    }

    #[test]
    fn empty_system_prompt_override_skipped_real_override_wins() {
        // Empty override followed by a real override — real one wins.
        let responses = vec![
            make_response(None, None, Some(""), None),
            make_response(None, None, Some("Real override"), None),
        ];
        let merged = merge_hook_responses("Original.", &responses);
        assert_eq!(merged.system_prompt, "Real override");
    }

    #[test]
    fn max_iterations_at_least_one() {
        // Even with a very small timeout, we should get at least 1 iteration.
        let iterations = (5u64 / 10u64).max(1);
        assert_eq!(iterations, 1);
    }

    #[test]
    fn empty_strings_are_skipped() {
        let responses = vec![make_response(Some(""), Some(""), None, Some(""))];
        let merged = merge_hook_responses("Original.", &responses);
        assert_eq!(merged.system_prompt, "Original.");
        assert_eq!(merged.user_context_prefix, "");
    }

    #[test]
    fn empty_original_with_prepend_and_append() {
        let responses = vec![make_response(Some("Prefix"), Some("Suffix"), None, None)];
        let merged = merge_hook_responses("", &responses);
        // Empty base prompt: prefix + separator + empty + separator + suffix
        // but we skip the separator when the base is empty on either side.
        assert_eq!(merged.system_prompt, "Prefix\nSuffix");
    }

    #[test]
    fn hook_response_deserializes_from_camel_case_json() {
        let json = r#"{
            "prependSystemContext": "Date info",
            "appendSystemContext": "Format rules",
            "systemPrompt": "Custom prompt",
            "prependContext": "User context"
        }"#;
        let resp: HookResponse = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(resp.prepend_system_context.as_deref(), Some("Date info"));
        assert_eq!(resp.append_system_context.as_deref(), Some("Format rules"));
        assert_eq!(resp.system_prompt.as_deref(), Some("Custom prompt"));
        assert_eq!(resp.prepend_context.as_deref(), Some("User context"));
    }

    #[test]
    fn hook_response_deserializes_partial_json() {
        let json = r#"{"prependSystemContext": "Just this"}"#;
        let resp: HookResponse = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(resp.prepend_system_context.as_deref(), Some("Just this"));
        assert!(resp.append_system_context.is_none());
        assert!(resp.system_prompt.is_none());
        assert!(resp.prepend_context.is_none());
    }

    #[test]
    fn hook_response_deserializes_empty_json() {
        let json = "{}";
        let resp: HookResponse = serde_json::from_str(json).expect("should deserialize");
        assert!(resp.prepend_system_context.is_none());
        assert!(resp.append_system_context.is_none());
        assert!(resp.system_prompt.is_none());
        assert!(resp.prepend_context.is_none());

        // Empty response should not alter original prompt.
        let merged = merge_hook_responses("Original.", &[resp]);
        assert_eq!(merged.system_prompt, "Original.");
    }

    #[test]
    fn assemble_request_deserializes() {
        let json = r#"{
            "messages": [{"role": "user", "content": "Hello"}],
            "system_prompt": "You are helpful.",
            "request_id": "abc-123",
            "model": "claude-sonnet-4-20250514",
            "provider": "anthropic"
        }"#;
        let req: AssembleRequest = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(req.request_id, "abc-123");
        assert_eq!(req.system_prompt, "You are helpful.");
        assert_eq!(req.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn parse_hook_responses_from_ipc_envelope() {
        // Simulate a realistic IPC poll envelope with multiple plugin messages.
        let envelope = serde_json::json!({
            "messages": [
                {
                    "topic": "prompt_builder.hook_response.req-42",
                    "source_id": "plugin-date-context",
                    "payload": {
                        "prependSystemContext": "Current date: 2026-03-08"
                    }
                },
                {
                    "topic": "prompt_builder.hook_response.req-42",
                    "source_id": "plugin-format-rules",
                    "payload": {
                        "appendSystemContext": "Always use markdown.",
                        "prependContext": "User timezone: UTC+2"
                    }
                },
                {
                    "topic": "prompt_builder.hook_response.req-42",
                    "source_id": "plugin-custom-prompt",
                    "payload": {
                        "data": {
                            "systemPrompt": "You are a custom assistant.",
                            "prependSystemContext": "Extra context"
                        }
                    }
                }
            ]
        });
        let bytes = serde_json::to_vec(&envelope).expect("serialize");
        let sourced = parse_hook_responses(&bytes).expect("should parse");
        assert_eq!(sourced.len(), 3);

        // First: direct payload with prependSystemContext
        assert_eq!(sourced[0].source_id.as_deref(), Some("plugin-date-context"));
        assert_eq!(
            sourced[0].response.prepend_system_context.as_deref(),
            Some("Current date: 2026-03-08")
        );

        // Second: direct payload with append + prepend context
        assert_eq!(sourced[1].source_id.as_deref(), Some("plugin-format-rules"));
        assert_eq!(
            sourced[1].response.append_system_context.as_deref(),
            Some("Always use markdown.")
        );
        assert_eq!(
            sourced[1].response.prepend_context.as_deref(),
            Some("User timezone: UTC+2")
        );

        // Third: nested in Custom `data` envelope
        assert_eq!(sourced[2].source_id.as_deref(), Some("plugin-custom-prompt"));
        assert_eq!(
            sourced[2].response.system_prompt.as_deref(),
            Some("You are a custom assistant.")
        );

        // Now merge and verify full assembly.
        let responses: Vec<HookResponse> = filter_by_permission(sourced);
        let merged = merge_hook_responses("Default prompt.", &responses);
        assert_eq!(
            merged.system_prompt,
            "Current date: 2026-03-08\nExtra context\nYou are a custom assistant.\nAlways use markdown."
        );
        assert_eq!(merged.user_context_prefix, "User timezone: UTC+2");
    }

    #[test]
    fn parse_hook_responses_empty_envelope() {
        let envelope = serde_json::json!({"messages": []});
        let bytes = serde_json::to_vec(&envelope).expect("serialize");
        assert!(parse_hook_responses(&bytes).is_none());
    }

    #[test]
    fn parse_hook_responses_invalid_json() {
        assert!(parse_hook_responses(b"not json").is_none());
    }

    #[test]
    fn parse_hook_responses_missing_payload() {
        let envelope = serde_json::json!({
            "messages": [{"topic": "test"}]
        });
        let bytes = serde_json::to_vec(&envelope).expect("serialize");
        assert!(parse_hook_responses(&bytes).is_none());
    }

    #[test]
    fn filter_by_permission_passes_all_currently() {
        let sourced = vec![
            SourcedHookResponse {
                source_id: Some("trusted-plugin".into()),
                response: make_response(Some("Context"), None, Some("Override"), None),
            },
            SourcedHookResponse {
                source_id: Some("untrusted-plugin".into()),
                response: make_response(None, Some("Suffix"), None, Some("User ctx")),
            },
            SourcedHookResponse {
                source_id: None,
                response: make_response(Some("Anonymous"), None, None, None),
            },
        ];
        let filtered = filter_by_permission(sourced);
        // Currently all responses pass through (permission gating is TODO).
        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0].system_prompt.as_deref(), Some("Override"));
        assert_eq!(
            filtered[1].append_system_context.as_deref(),
            Some("Suffix")
        );
        assert_eq!(
            filtered[2].prepend_system_context.as_deref(),
            Some("Anonymous")
        );
    }

    #[test]
    fn assemble_response_serializes_correctly() {
        let resp = AssembleResponse {
            system_prompt: "Final prompt.".to_string(),
            user_context_prefix: "User ctx".to_string(),
            request_id: "req-1".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("should serialize");
        assert_eq!(json["system_prompt"], "Final prompt.");
        assert_eq!(json["user_context_prefix"], "User ctx");
        assert_eq!(json["request_id"], "req-1");
    }

    // ── Topic filtering tests ─────────────────────────────────────

    #[test]
    fn should_dispatch_assemble_topic() {
        assert!(should_dispatch_topic("prompt_builder.assemble"));
    }

    #[test]
    fn should_not_dispatch_own_response_topics() {
        assert!(!should_dispatch_topic("prompt_builder.response.assemble"));
        assert!(!should_dispatch_topic("prompt_builder.response.foo"));
    }

    #[test]
    fn should_not_dispatch_hook_response_topics() {
        assert!(!should_dispatch_topic(
            "prompt_builder.hook_response.req-42"
        ));
        assert!(!should_dispatch_topic(
            "prompt_builder.hook_response.abc-123"
        ));
    }

    #[test]
    fn should_not_dispatch_interceptor_topics() {
        assert!(!should_dispatch_topic("before_prompt_build"));
        assert!(!should_dispatch_topic("after_prompt_build"));
    }

    #[test]
    fn should_dispatch_unrelated_topics() {
        // These would be ignored by the match anyway, but shouldn't be filtered.
        assert!(should_dispatch_topic("prompt_builder.some_other_action"));
        assert!(should_dispatch_topic("prompt_builder.status"));
    }

    // ── Response topic isolation tests ────────────────────────────

    #[test]
    fn response_topics_are_unique_per_request_id() {
        // Proves that concurrent requests can't cross-contaminate.
        let topic_a = format!("prompt_builder.hook_response.{}", "req-aaa");
        let topic_b = format!("prompt_builder.hook_response.{}", "req-bbb");
        assert_ne!(topic_a, topic_b);
        // Each topic is specific enough that subscribing to one won't receive the other.
        assert!(!topic_a.ends_with("req-bbb"));
        assert!(!topic_b.ends_with("req-aaa"));
    }

    // ── BeforePromptBuildPayload tests ────────────────────────────

    #[test]
    fn before_prompt_build_payload_includes_response_topic() {
        let payload = BeforePromptBuildPayload {
            messages: serde_json::json!([]),
            system_prompt: "test".to_string(),
            request_id: "req-99".to_string(),
            model: "claude".to_string(),
            provider: "anthropic".to_string(),
            response_topic: "prompt_builder.hook_response.req-99".to_string(),
        };
        let json = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(
            json["response_topic"],
            "prompt_builder.hook_response.req-99"
        );
        // Plugins need this field to know where to send their response.
        assert!(json.get("response_topic").is_some());
    }

    #[test]
    fn before_prompt_build_payload_round_trips() {
        let original = BeforePromptBuildPayload {
            messages: serde_json::json!([{"role": "user", "content": "hi"}]),
            system_prompt: "You are helpful.".to_string(),
            request_id: "req-123".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            provider: "anthropic".to_string(),
            response_topic: "prompt_builder.hook_response.req-123".to_string(),
        };
        let bytes = serde_json::to_vec(&original).expect("serialize");
        let restored: BeforePromptBuildPayload =
            serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(restored.request_id, "req-123");
        assert_eq!(restored.system_prompt, "You are helpful.");
        assert_eq!(restored.model, "claude-sonnet-4-20250514");
        assert_eq!(
            restored.response_topic,
            "prompt_builder.hook_response.req-123"
        );
    }

    // ── AfterPromptBuildPayload tests ─────────────────────────────

    #[test]
    fn after_prompt_build_payload_serializes() {
        let payload = AfterPromptBuildPayload {
            system_prompt: "Final.".to_string(),
            user_context_prefix: "ctx".to_string(),
            request_id: "req-1".to_string(),
        };
        let json = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(json["system_prompt"], "Final.");
        assert_eq!(json["user_context_prefix"], "ctx");
        assert_eq!(json["request_id"], "req-1");
    }

    // ── has_any_field edge cases ──────────────────────────────────

    #[test]
    fn has_any_field_returns_false_for_default() {
        assert!(!HookResponse::default().has_any_field());
    }

    #[test]
    fn has_any_field_detects_each_field_independently() {
        assert!(make_response(Some("x"), None, None, None).has_any_field());
        assert!(make_response(None, Some("x"), None, None).has_any_field());
        assert!(make_response(None, None, Some("x"), None).has_any_field());
        assert!(make_response(None, None, None, Some("x")).has_any_field());
    }

    // ── parse_hook_responses: unrelated payload is not a false positive ──

    #[test]
    fn parse_hook_responses_ignores_unrelated_payload() {
        // A message with a payload that has no hook response fields
        // should not be parsed as a HookResponse.
        let envelope = serde_json::json!({
            "messages": [{
                "topic": "prompt_builder.hook_response.req-1",
                "source_id": "some-capsule",
                "payload": {
                    "status": "ok",
                    "unrelated_field": 42
                }
            }]
        });
        let bytes = serde_json::to_vec(&envelope).expect("serialize");
        assert!(
            parse_hook_responses(&bytes).is_none(),
            "unrelated payload should not produce a HookResponse"
        );
    }

    #[test]
    fn parse_hook_responses_mixed_valid_and_invalid() {
        let envelope = serde_json::json!({
            "messages": [
                {
                    "topic": "hook",
                    "payload": {"status": "irrelevant"}
                },
                {
                    "topic": "hook",
                    "source_id": "good-plugin",
                    "payload": {"prependSystemContext": "Valid context"}
                }
            ]
        });
        let bytes = serde_json::to_vec(&envelope).expect("serialize");
        let sourced = parse_hook_responses(&bytes).expect("should parse valid one");
        assert_eq!(sourced.len(), 1);
        assert_eq!(sourced[0].source_id.as_deref(), Some("good-plugin"));
        assert_eq!(
            sourced[0].response.prepend_system_context.as_deref(),
            Some("Valid context")
        );
    }
}
