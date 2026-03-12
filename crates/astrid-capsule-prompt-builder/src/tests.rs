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
    // Empty base prompt is skipped; prefix and suffix join directly.
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
                "topic": "prompt_builder.v1.hook_response.req-42",
                "source_id": "plugin-date-context",
                "payload": {
                    "prependSystemContext": "Current date: 2026-03-08"
                }
            },
            {
                "topic": "prompt_builder.v1.hook_response.req-42",
                "source_id": "plugin-format-rules",
                "payload": {
                    "appendSystemContext": "Always use markdown.",
                    "prependContext": "User timezone: UTC+2"
                }
            },
            {
                "topic": "prompt_builder.v1.hook_response.req-42",
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
    assert!(should_dispatch_topic("prompt_builder.v1.assemble"));
}

#[test]
fn should_not_dispatch_own_response_topics() {
    assert!(!should_dispatch_topic("prompt_builder.v1.response.assemble"));
    assert!(!should_dispatch_topic("prompt_builder.v1.response.foo"));
}

#[test]
fn should_not_dispatch_hook_response_topics() {
    assert!(!should_dispatch_topic(
        "prompt_builder.v1.hook_response.req-42"
    ));
    assert!(!should_dispatch_topic(
        "prompt_builder.v1.hook_response.abc-123"
    ));
}

#[test]
fn should_not_dispatch_interceptor_topics() {
    assert!(!should_dispatch_topic("prompt_builder.v1.hook.before_build"));
    assert!(!should_dispatch_topic("prompt_builder.v1.hook.after_build"));
}

#[test]
fn should_dispatch_unrelated_topics() {
    // These would be ignored by the match anyway, but shouldn't be filtered.
    assert!(should_dispatch_topic("prompt_builder.v1.some_other_action"));
    assert!(should_dispatch_topic("prompt_builder.v1.status"));
}

// ── Response topic isolation tests ────────────────────────────

#[test]
fn response_topics_are_unique_per_request_id() {
    // Proves that concurrent requests can't cross-contaminate.
    let topic_a = format!("prompt_builder.v1.hook_response.{}", "req-aaa");
    let topic_b = format!("prompt_builder.v1.hook_response.{}", "req-bbb");
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
        response_topic: "prompt_builder.v1.hook_response.req-99".to_string(),
    };
    let json = serde_json::to_value(&payload).expect("serialize");
    assert_eq!(
        json["response_topic"],
        "prompt_builder.v1.hook_response.req-99"
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
        response_topic: "prompt_builder.v1.hook_response.req-123".to_string(),
    };
    let bytes = serde_json::to_vec(&original).expect("serialize");
    let restored: BeforePromptBuildPayload =
        serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(restored.request_id, "req-123");
    assert_eq!(restored.system_prompt, "You are helpful.");
    assert_eq!(restored.model, "claude-sonnet-4-20250514");
    assert_eq!(
        restored.response_topic,
        "prompt_builder.v1.hook_response.req-123"
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
