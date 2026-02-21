//! Pure helper functions for inbound message processing.
//!
//! All functions here are used exclusively by `handler.rs`
//! (`handle_inbound_message`, `register_channels_locally`).

#[cfg(test)]
use astrid_core::ConnectorCapabilities;
use astrid_core::FrontendType;
use serde_json::Value;
use tracing::warn;

/// Maximum length for a custom platform name (128 bytes).
///
/// Platform names that exceed this limit are truncated at a UTF-8 character
/// boundary. Known platform names (discord, telegram, etc.) are never
/// affected since they are matched before the custom fallback.
pub(super) const MAX_PLATFORM_NAME_BYTES: usize = 128;

/// Map a platform name string to a [`FrontendType`].
///
/// Custom platform names are truncated to [`MAX_PLATFORM_NAME_BYTES`].
pub(super) fn map_platform_name(name: &str) -> FrontendType {
    match name.to_lowercase().as_str() {
        "telegram" => FrontendType::Telegram,
        "discord" => FrontendType::Discord,
        "slack" => FrontendType::Slack,
        "whatsapp" => FrontendType::WhatsApp,
        "web" => FrontendType::Web,
        "cli" => FrontendType::Cli,
        other => {
            let truncated = if other.len() > MAX_PLATFORM_NAME_BYTES {
                &other[..other.floor_char_boundary(MAX_PLATFORM_NAME_BYTES)]
            } else {
                other
            };
            FrontendType::Custom(truncated.to_string())
        },
    }
}

/// Parse connector capabilities from a channel definition JSON object.
///
/// Looks for `chatTypes` or `capabilities` arrays in the definition and maps
/// known strings to capability flags. Falls back to `receive_only()`.
///
/// Recognized strings (case-insensitive):
/// - `"receive"`, `"inbound"` → `can_receive`
/// - `"send"`, `"outbound"` → `can_send`
/// - `"chat"` → both `can_receive` and `can_send` (bidirectional)
/// - `"approve"` → `can_approve`
///
/// Other strings and non-string array elements are silently ignored.
/// Fields like `can_elicit`, `supports_rich_media`, `supports_threads`,
/// and `supports_buttons` are not yet parsed from the bridge definition
/// and default to `false`.
#[cfg(test)]
pub(super) fn parse_connector_capabilities(definition: &Value) -> ConnectorCapabilities {
    let caps_array = definition
        .get("capabilities")
        .or_else(|| definition.get("chatTypes"))
        .and_then(Value::as_array);

    let Some(arr) = caps_array else {
        return ConnectorCapabilities::receive_only();
    };

    let lowered: Vec<String> = arr
        .iter()
        .take(64) // cap allocation against adversarial arrays
        .filter_map(Value::as_str)
        .map(str::to_lowercase)
        .collect();
    if lowered.is_empty() {
        return ConnectorCapabilities::receive_only();
    }

    let can_receive = lowered
        .iter()
        .any(|s| s == "receive" || s == "inbound" || s == "chat");
    let can_send = lowered
        .iter()
        .any(|s| s == "send" || s == "outbound" || s == "chat");
    let can_approve = lowered.iter().any(|s| s == "approve");

    // If we parsed something meaningful, build from flags; otherwise receive_only
    if can_receive || can_send || can_approve {
        ConnectorCapabilities {
            can_receive,
            can_send,
            can_approve,
            ..ConnectorCapabilities::default()
        }
    } else {
        ConnectorCapabilities::receive_only()
    }
}

/// Extract `platform_user_id` from an inbound message context JSON, with
/// fallback chain: `context.from.id` → `context.senderId` → `context.userId`
/// → `"unknown"`. Truncated to [`super::notice::MAX_PLATFORM_USER_ID_BYTES`]
/// at a valid UTF-8 character boundary.
pub(super) fn extract_platform_user_id(context: &Value, max_bytes: usize) -> String {
    let raw = context
        .get("from")
        .and_then(|f| f.get("id"))
        .and_then(Value::as_str)
        .or_else(|| context.get("senderId").and_then(Value::as_str))
        .or_else(|| context.get("userId").and_then(Value::as_str))
        .unwrap_or("unknown");

    if raw.len() > max_bytes {
        // Truncate at a valid UTF-8 character boundary to avoid panics
        // on multi-byte characters that straddle the limit.
        let boundary = raw.floor_char_boundary(max_bytes);
        raw[..boundary].to_string()
    } else {
        raw.to_string()
    }
}

/// Extract and validate the `content` field from inbound message params.
///
/// Returns `None` (with a warning) if content is missing, null, empty, or
/// exceeds `max_bytes`. Non-string values are serialized to JSON with a
/// post-serialization size check to guard against escape-amplification attacks.
pub(super) fn extract_inbound_content(params: &Value, max_bytes: usize) -> Option<String> {
    let Some(content_val) = params.get("content").filter(|v| !v.is_null()) else {
        warn!("inboundMessage: missing or null content");
        return None;
    };
    if let Some(s) = content_val.as_str() {
        if s.is_empty() || s.len() > max_bytes {
            warn!("inboundMessage: string content empty or exceeds size limit");
            return None;
        }
        Some(s.to_string())
    } else {
        // Non-string content (objects, arrays) is serialized. Check the
        // expanded size to guard against escape-amplification attacks
        // (e.g. control chars expanding 1 byte → 6 bytes as \uNNNN).
        let serialized = content_val.to_string();
        if serialized.len() > max_bytes {
            warn!("inboundMessage: serialized content exceeds limit after expansion");
            return None;
        }
        Some(serialized)
    }
}

/// Estimate the serialized JSON size of a [`Value`] by walking the parsed tree.
///
/// Counts bytes for keys, string values, structural characters, and numeric
/// representations. **Known limitation:** strings containing JSON-escaped
/// characters (control chars `\x00`–`\x1f`, backslashes, quotes) can expand
/// up to 6× when re-serialized (e.g. `\x00` → `\u0000`). This means the
/// estimate may *undercount* by up to 6× in adversarial payloads.
///
/// # Recursion safety
///
/// This function recurses into nested arrays and objects. Its stack depth is
/// bounded by `serde_json`'s default recursion limit (128 levels), which is
/// applied during parsing.
pub(super) fn estimate_json_size(value: &Value) -> usize {
    match value {
        Value::Null => 4, // "null"
        Value::Bool(b) => {
            if *b { 4 } else { 5 } // "true" / "false"
        },
        Value::Number(n) => {
            // Allocates briefly for digit count; accurate for small numbers.
            n.to_string().len()
        },
        Value::String(s) => {
            // 2 for quotes + string length (ignoring escape expansion)
            s.len().saturating_add(2)
        },
        Value::Array(arr) => {
            // 2 for [] + commas + recursive sizes
            let inner: usize = arr.iter().map(estimate_json_size).sum();
            let commas = arr.len().saturating_sub(1);
            inner.saturating_add(2).saturating_add(commas)
        },
        Value::Object(map) => {
            // 2 for {} + commas + key/value pairs
            let inner: usize = map
                .iter()
                .map(|(k, v)| {
                    // key: 2 quotes + len + colon + value
                    k.len()
                        .saturating_add(3)
                        .saturating_add(estimate_json_size(v))
                })
                .sum();
            let commas = map.len().saturating_sub(1);
            inner.saturating_add(2).saturating_add(commas)
        },
    }
}

#[cfg(test)]
#[allow(clippy::cast_possible_wrap)]
mod tests {
    use super::*;
    use crate::capabilities::client::notice::MAX_PLATFORM_USER_ID_BYTES;

    #[test]
    fn test_map_platform_name() {
        assert!(matches!(
            map_platform_name("Telegram"),
            FrontendType::Telegram
        ));
        assert!(matches!(
            map_platform_name("DISCORD"),
            FrontendType::Discord
        ));
        assert!(matches!(map_platform_name("slack"), FrontendType::Slack));
        assert!(matches!(
            map_platform_name("WhatsApp"),
            FrontendType::WhatsApp
        ));
        assert!(matches!(map_platform_name("web"), FrontendType::Web));
        assert!(matches!(map_platform_name("cli"), FrontendType::Cli));
        assert!(matches!(
            map_platform_name("matrix"),
            FrontendType::Custom(_)
        ));
        if let FrontendType::Custom(name) = map_platform_name("Matrix") {
            assert_eq!(name, "matrix");
        }
    }

    #[test]
    fn test_map_platform_name_empty_string() {
        let ft = map_platform_name("");
        assert!(matches!(ft, FrontendType::Custom(ref s) if s.is_empty()));
    }

    #[test]
    fn test_map_platform_name_long_custom_truncated() {
        let long_name = "x".repeat(300);
        let ft = map_platform_name(&long_name);
        if let FrontendType::Custom(s) = ft {
            assert!(
                s.len() <= MAX_PLATFORM_NAME_BYTES,
                "custom platform name should be truncated to {MAX_PLATFORM_NAME_BYTES}, got {}",
                s.len()
            );
        } else {
            panic!("expected Custom variant");
        }
    }

    #[test]
    fn test_parse_connector_capabilities_chat() {
        let def = serde_json::json!({ "capabilities": ["receive", "send", "approve"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
        assert!(caps.can_approve);
    }

    #[test]
    fn test_parse_connector_capabilities_fallback() {
        let def = serde_json::json!({});
        let caps = parse_connector_capabilities(&def);
        assert_eq!(caps, ConnectorCapabilities::receive_only());
    }

    #[test]
    fn test_parse_connector_capabilities_chat_types_key() {
        let def = serde_json::json!({ "chatTypes": ["receive", "send"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
        assert!(!caps.can_approve);
    }

    #[test]
    fn test_parse_connector_capabilities_chat_bidirectional() {
        let def = serde_json::json!({ "capabilities": ["chat"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
    }

    #[test]
    fn test_parse_connector_capabilities_non_string_elements_ignored() {
        let def = serde_json::json!({
            "capabilities": [42, true, null, "receive", "send"]
        });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
        assert!(!caps.can_approve);
    }

    #[test]
    fn test_parse_connector_capabilities_inbound_outbound_synonyms() {
        let def = serde_json::json!({ "capabilities": ["inbound", "outbound"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive, "inbound should set can_receive");
        assert!(caps.can_send, "outbound should set can_send");
        assert!(!caps.can_approve);
    }

    #[test]
    fn test_parse_connector_capabilities_unrecognized_strings_only() {
        let def = serde_json::json!({ "capabilities": ["foo", "bar", "baz"] });
        let caps = parse_connector_capabilities(&def);
        assert_eq!(caps, ConnectorCapabilities::receive_only());
    }

    #[test]
    fn test_parse_connector_capabilities_all_non_string_elements() {
        let def = serde_json::json!({ "capabilities": [42, true, null, [1, 2]] });
        let caps = parse_connector_capabilities(&def);
        assert_eq!(caps, ConnectorCapabilities::receive_only());
    }

    #[test]
    fn test_parse_connector_capabilities_capabilities_key_takes_priority() {
        let def = serde_json::json!({
            "capabilities": ["send"],
            "chatTypes": ["receive"]
        });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_send, "capabilities key should take priority");
        assert!(
            !caps.can_receive,
            "chatTypes should be ignored when capabilities is present"
        );
    }

    #[test]
    fn test_extract_platform_user_id_from_id() {
        let ctx = serde_json::json!({ "from": { "id": "user-42" } });
        assert_eq!(
            extract_platform_user_id(&ctx, MAX_PLATFORM_USER_ID_BYTES),
            "user-42"
        );
    }

    #[test]
    fn test_extract_platform_user_id_sender_id() {
        let ctx = serde_json::json!({ "senderId": "sender-99" });
        assert_eq!(
            extract_platform_user_id(&ctx, MAX_PLATFORM_USER_ID_BYTES),
            "sender-99"
        );
    }

    #[test]
    fn test_extract_platform_user_id_fallback() {
        let ctx = serde_json::json!({});
        assert_eq!(
            extract_platform_user_id(&ctx, MAX_PLATFORM_USER_ID_BYTES),
            "unknown"
        );
    }

    #[test]
    fn test_extract_platform_user_id_truncated() {
        let long_id = "x".repeat(MAX_PLATFORM_USER_ID_BYTES + 100);
        let ctx = serde_json::json!({ "senderId": long_id });
        let result = extract_platform_user_id(&ctx, MAX_PLATFORM_USER_ID_BYTES);
        assert_eq!(result.len(), MAX_PLATFORM_USER_ID_BYTES);
    }

    #[test]
    fn test_extract_platform_user_id_multibyte_truncation() {
        let emoji = "\u{1F600}"; // 4 bytes each
        let count = MAX_PLATFORM_USER_ID_BYTES / emoji.len() + 5;
        let long_id: String = emoji.repeat(count);
        assert!(long_id.len() > MAX_PLATFORM_USER_ID_BYTES);

        let ctx = serde_json::json!({ "senderId": long_id });
        let result = extract_platform_user_id(&ctx, MAX_PLATFORM_USER_ID_BYTES);
        assert!(result.len() <= MAX_PLATFORM_USER_ID_BYTES);
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_extract_platform_user_id_exactly_at_limit() {
        let id_at_limit = "x".repeat(MAX_PLATFORM_USER_ID_BYTES);
        let ctx = serde_json::json!({ "senderId": id_at_limit });
        let result = extract_platform_user_id(&ctx, MAX_PLATFORM_USER_ID_BYTES);
        assert_eq!(result.len(), MAX_PLATFORM_USER_ID_BYTES);
        assert_eq!(result, id_at_limit);
    }

    #[test]
    fn test_extract_platform_user_id_userid_key() {
        let ctx = serde_json::json!({ "userId": "user-from-userid" });
        let id = extract_platform_user_id(&ctx, MAX_PLATFORM_USER_ID_BYTES);
        assert_eq!(id, "user-from-userid");
    }

    #[test]
    fn test_estimate_json_size_primitives() {
        assert_eq!(estimate_json_size(&Value::Null), 4);
        assert_eq!(estimate_json_size(&Value::Bool(true)), 4);
        assert_eq!(estimate_json_size(&Value::Bool(false)), 5);
    }

    #[test]
    fn test_estimate_json_size_string() {
        let val = Value::String("hello".to_string());
        assert_eq!(estimate_json_size(&val), 7);
    }

    #[test]
    fn test_estimate_json_size_object() {
        let val = serde_json::json!({"a": 1});
        let size = estimate_json_size(&val);
        let actual = serde_json::to_string(&val).unwrap().len();
        assert!(size > 0);
        assert!(
            (size as i64 - actual as i64).unsigned_abs() <= 1,
            "estimate {size} should be within 1 of actual {actual}"
        );
    }

    #[test]
    fn test_estimate_json_size_large_payload() {
        use crate::capabilities::client::notice::MAX_NOTIFICATION_PAYLOAD_BYTES;
        let big = "x".repeat(MAX_NOTIFICATION_PAYLOAD_BYTES + 100);
        let val = serde_json::json!({ "data": big });
        assert!(estimate_json_size(&val) > MAX_NOTIFICATION_PAYLOAD_BYTES);
    }

    #[test]
    fn test_estimate_json_size_array() {
        let val = serde_json::json!([1, 2, 3]);
        let size = estimate_json_size(&val);
        let actual = serde_json::to_string(&val).unwrap().len();
        assert!(
            (size as i64 - actual as i64).unsigned_abs() <= 1,
            "array estimate {size} should be within 1 of actual {actual}"
        );
    }

    #[test]
    fn test_estimate_json_size_nested_array() {
        let val = serde_json::json!([[1], [2, 3]]);
        let size = estimate_json_size(&val);
        let actual = serde_json::to_string(&val).unwrap().len();
        assert!(
            (size as i64 - actual as i64).unsigned_abs() <= 2,
            "nested array estimate {size} should be within 2 of actual {actual}"
        );
    }

    #[test]
    fn test_estimate_json_size_empty_containers() {
        assert_eq!(estimate_json_size(&Value::String(String::new())), 2);
        assert_eq!(estimate_json_size(&serde_json::json!([])), 2);
        assert_eq!(estimate_json_size(&serde_json::json!({})), 2);
    }
}
