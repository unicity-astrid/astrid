#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]

//! Prompt Builder capsule — assembles LLM prompts with plugin hook interception.
//!
//! This capsule owns the prompt assembly pipeline. When the react loop needs
//! a prompt assembled, it publishes to `prompt_builder.v1.assemble`. The prompt
//! builder then:
//!
//! 1. Fires `prompt_builder.v1.hook.before_build` to all plugin capsules via IPC
//! 2. Collects plugin responses (`prependSystemContext`, `appendSystemContext`,
//!    `systemPrompt` override, `prependContext`)
//! 3. Merges them according to OpenClaw-compatible semantics
//! 4. Returns the assembled prompt on `prompt_builder.v1.response.assemble`
//! 5. Fires `prompt_builder.v1.hook.after_build` as a notification
//!
//! # Merge Semantics
//!
//! 1. `prependContext` — concatenated in order, becomes `user_context_prefix`
//! 2. `systemPrompt` — last non-null value wins (full override)
//! 3. `prependSystemContext` — concatenated in order, prepended to system prompt
//! 4. `appendSystemContext` — concatenated in order, appended to system prompt

use astrid_sdk::prelude::*;
use extism_pdk::{plugin_fn, FnResult};
use serde::{Deserialize, Serialize};

/// Runtime configuration loaded from capsule config at startup.
struct Config {
    /// Maximum time (in milliseconds) to wait for plugin hook responses.
    hook_timeout_ms: u64,
}

impl Config {
    /// Load configuration from the capsule's config store, falling back to defaults.
    fn load() -> Self {
        let hook_timeout_ms = env::var("hook_timeout_ms")
            .ok()
            .and_then(|s| s.trim().trim_matches('"').parse::<u64>().ok())
            .unwrap_or(DEFAULT_HOOK_POLL_TIMEOUT_MS);

        Self { hook_timeout_ms }
    }
}

/// Request from the react loop to assemble a prompt.
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
    /// Session ID echoed back for react loop correlation.
    #[serde(default)]
    session_id: Option<String>,
}

/// Response returned to the react loop with the assembled prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AssembleResponse {
    /// The final assembled system prompt.
    system_prompt: String,
    /// Text to prepend to the user's message (from `prependContext` hooks).
    user_context_prefix: String,
    /// The original request ID for correlation.
    request_id: String,
    /// Session ID echoed from the request for react loop correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

/// Payload sent to plugin capsules via the `prompt_builder.v1.hook.before_build` interceptor.
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

/// A single plugin's response to the `prompt_builder.v1.hook.before_build` hook.
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

    // Step 3-4: Prepend + base + append, joined with newlines.
    let mut parts: Vec<&str> = Vec::new();
    parts.extend_from_slice(&prepend_system_contexts);
    if !base_prompt.is_empty() {
        parts.push(base_prompt);
    }
    parts.extend_from_slice(&append_system_contexts);
    let final_prompt = parts.join("\n");

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

/// Fire the `prompt_builder.v1.hook.before_build` interceptor and collect plugin responses.
///
/// Publishes the hook event on the `prompt_builder.v1.hook.before_build` IPC topic and polls
/// a dedicated response topic for plugin contributions. Returns all collected
/// responses within the timeout window, filtered by permission gating.
fn fire_before_prompt_build(request: &AssembleRequest, config: &Config) -> Vec<HookResponse> {
    let response_topic = format!(
        "prompt_builder.v1.hook_response.{}",
        request.request_id
    );

    // Subscribe BEFORE publishing to avoid missing fast responses.
    let sub = match ipc::subscribe(&response_topic) {
        Ok(h) => h,
        Err(e) => {
            let _ = log::log(
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

    if let Err(e) = ipc::publish_json("prompt_builder.v1.hook.before_build", &payload) {
        let _ = log::log(
            "error",
            format!("Failed to publish prompt_builder.v1.hook.before_build event: {e}"),
        );
        let _ = ipc::unsubscribe(&sub);
        return Vec::new();
    }

    // Block-wait for hook responses within the configured timeout.
    let mut sourced_responses = Vec::new();
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(config.hook_timeout_ms);

    while std::time::Instant::now() < deadline && sourced_responses.len() < MAX_HOOK_RESPONSES {
        let remaining_ms = deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis();
        if remaining_ms == 0 {
            break;
        }
        let timeout = u64::try_from(remaining_ms).unwrap_or(u64::MAX);

        match ipc::recv_bytes(&sub, timeout) {
            Ok(bytes) if !bytes.is_empty() => {
                if let Some(new_responses) = parse_hook_responses(&bytes) {
                    sourced_responses.extend(new_responses);
                }
            },
            _ => break,
        }
    }

    let _ = ipc::unsubscribe(&sub);

    let _ = log::log(
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
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(e) => {
            let _ = log::log(
                "warn",
                format!("failed to deserialize hook response envelope: {e}"),
            );
            return None;
        },
    };

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

/// Fire the `prompt_builder.v1.hook.after_build` notification (fire-and-forget).
fn fire_after_prompt_build(system_prompt: &str, user_context_prefix: &str, request_id: &str) {
    let payload = AfterPromptBuildPayload {
        system_prompt: system_prompt.to_string(),
        user_context_prefix: user_context_prefix.to_string(),
        request_id: request_id.to_string(),
    };
    let _ = ipc::publish_json("prompt_builder.v1.hook.after_build", &payload);
}

/// Handle a single `prompt_builder.v1.assemble` request.
fn handle_assemble(payload: &serde_json::Value, config: &Config) {
    // Extract from Custom payload envelope or direct.
    let request_value = payload
        .get("data")
        .unwrap_or(payload);

    let request: AssembleRequest = match serde_json::from_value(request_value.clone()) {
        Ok(r) => r,
        Err(e) => {
            let _ = log::log(
                "error",
                format!("Failed to parse assemble request: {e}"),
            );
            let _ = ipc::publish_json(
                "prompt_builder.v1.response.assemble",
                &serde_json::json!({"error": format!("invalid request: {e}")}),
            );
            return;
        },
    };

    if request.request_id.is_empty() {
        let _ = ipc::publish_json(
            "prompt_builder.v1.response.assemble",
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
        session_id: request.session_id.clone(),
    };

    let _ = ipc::publish_json("prompt_builder.v1.response.assemble", &response);

    // Fire after_prompt_build notification (fire-and-forget).
    fire_after_prompt_build(&merged.system_prompt, &merged.user_context_prefix, &request.request_id);
}

/// Returns `true` if the topic should be dispatched (not a self-echo).
///
/// Filters out our own response topics, hook response topics, and the
/// interceptor topics we publish. Only `prompt_builder.v1.assemble` passes.
fn should_dispatch_topic(topic: &str) -> bool {
    !topic.starts_with("prompt_builder.v1.response.")
        && !topic.starts_with("prompt_builder.v1.hook_response.")
        && topic != "prompt_builder.v1.hook.before_build"
        && topic != "prompt_builder.v1.hook.after_build"
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
        let _ = log::log(
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

        if topic == "prompt_builder.v1.assemble"
            && let Some(payload) = msg.get("payload")
        {
            handle_assemble(payload, config);
        }
    }
}

#[plugin_fn]
pub fn run() -> FnResult<()> {
    let _ = log::info("Prompt Builder capsule starting");

    let config = Config::load();
    let _ = log::info(format!("Hook timeout: {}ms", config.hook_timeout_ms));

    let sub = ipc::subscribe("prompt_builder.v1.*")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // Also subscribe to our own hook topics so we can filter them out.
    let hook_sub = ipc::subscribe("prompt_builder.v1.hook.before_build")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;
    let after_sub = ipc::subscribe("prompt_builder.v1.hook.after_build")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // Signal readiness so the kernel can proceed with loading dependent capsules.
    // Best-effort: failure means the host mutex is poisoned (unrecoverable).
    let _ = runtime::signal_ready();

    let _ = log::info("Prompt Builder capsule ready");

    loop {
        // Block until a message arrives (up to 60s), eliminating busy-spin polling.
        match ipc::recv_bytes(&sub, 60_000) {
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
    }

    let _ = ipc::unsubscribe(&sub);
    let _ = ipc::unsubscribe(&hook_sub);
    let _ = ipc::unsubscribe(&after_sub);

    Ok(())
}

#[cfg(test)]
mod tests;
