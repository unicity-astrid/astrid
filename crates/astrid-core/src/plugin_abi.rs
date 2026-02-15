//! Mirror Rust types for the WIT plugin ABI (`astrid:plugin@0.1.0`).
//!
//! These types are the shared vocabulary between the Astrid host and WASM
//! plugin guests. They mirror the records and enums defined in
//! `wit/astrid-plugin.wit` exactly, ensuring a single source of truth for
//! serialization formats.
//!
//! Used by:
//! - **WS-1** (Plugin traits) — function signatures
//! - **WS-3** (Extism integration) — host ↔ guest serialization
//! - **WS-5** (`OpenClaw` shim) — ABI translation layer

use serde::{Deserialize, Serialize};

/// Log severity level for structured plugin logging.
///
/// Maps to the WIT `log-level` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Verbose tracing information.
    Trace,
    /// Debug-level diagnostic information.
    Debug,
    /// General informational messages.
    Info,
    /// Warning conditions that may need attention.
    Warn,
    /// Error conditions.
    Error,
}

/// A key-value pair used for typed header lists.
///
/// Maps to the WIT `key-value-pair` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyValuePair {
    /// Header or metadata key.
    pub key: String,
    /// Header or metadata value.
    pub value: String,
}

/// Context passed to a plugin when a hook fires.
///
/// Maps to the WIT `plugin-context` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginContext {
    /// The event name that triggered this hook (e.g. `"pre-tool-call"`).
    pub event: String,
    /// Session ID for the current interaction.
    pub session_id: String,
    /// Authenticated user ID, if available.
    pub user_id: Option<String>,
    /// Event-specific payload as a JSON string.
    pub data: Option<String>,
}

/// Result returned by a plugin after hook execution.
///
/// Maps to the WIT `plugin-result` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginResult {
    /// Action directive (e.g. `"continue"`, `"abort"`, `"modify"`).
    pub action: String,
    /// Optional payload as a JSON string.
    pub data: Option<String>,
}

/// Input arguments for a tool invocation.
///
/// Maps to the WIT `tool-input` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInput {
    /// Tool name to invoke.
    pub name: String,
    /// Tool arguments as a JSON string.
    pub arguments: String,
}

/// Output from a tool execution.
///
/// Maps to the WIT `tool-output` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Result content as a JSON string.
    pub content: String,
    /// Whether this output represents an error.
    pub is_error: bool,
}

/// Metadata describing a tool that a plugin exposes.
///
/// Maps to the WIT `tool-definition` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub input_schema: String,
}

/// HTTP response returned by the host.
///
/// Maps to the WIT `http-response` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<KeyValuePair>,
    /// Response body.
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: serialize to JSON and back, asserting round-trip equality.
    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*value, back);
    }

    #[test]
    fn log_level_round_trip() {
        for level in [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ] {
            round_trip(&level);
        }
    }

    #[test]
    fn log_level_json_format() {
        assert_eq!(
            serde_json::to_string(&LogLevel::Trace).unwrap(),
            "\"trace\""
        );
        assert_eq!(
            serde_json::to_string(&LogLevel::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn key_value_pair_round_trip() {
        round_trip(&KeyValuePair {
            key: "Content-Type".into(),
            value: "application/json".into(),
        });
    }

    #[test]
    fn plugin_context_round_trip() {
        round_trip(&PluginContext {
            event: "pre-tool-call".into(),
            session_id: "sess-123".into(),
            user_id: Some("user-456".into()),
            data: Some(r#"{"tool":"read_file"}"#.into()),
        });
    }

    #[test]
    fn plugin_context_minimal_round_trip() {
        round_trip(&PluginContext {
            event: "post-response".into(),
            session_id: "sess-789".into(),
            user_id: None,
            data: None,
        });
    }

    #[test]
    fn plugin_result_round_trip() {
        round_trip(&PluginResult {
            action: "continue".into(),
            data: None,
        });
        round_trip(&PluginResult {
            action: "modify".into(),
            data: Some(r#"{"patched":true}"#.into()),
        });
    }

    #[test]
    fn tool_input_round_trip() {
        round_trip(&ToolInput {
            name: "search".into(),
            arguments: r#"{"query":"hello"}"#.into(),
        });
    }

    #[test]
    fn tool_output_round_trip() {
        round_trip(&ToolOutput {
            content: r#"{"results":[]}"#.into(),
            is_error: false,
        });
        round_trip(&ToolOutput {
            content: "not found".into(),
            is_error: true,
        });
    }

    #[test]
    fn tool_definition_round_trip() {
        round_trip(&ToolDefinition {
            name: "weather".into(),
            description: "Get current weather".into(),
            input_schema: r#"{"type":"object","properties":{"city":{"type":"string"}}}"#.into(),
        });
    }

    #[test]
    fn http_response_round_trip() {
        round_trip(&HttpResponse {
            status: 200,
            headers: vec![
                KeyValuePair {
                    key: "Content-Type".into(),
                    value: "text/plain".into(),
                },
                KeyValuePair {
                    key: "X-Request-Id".into(),
                    value: "abc-123".into(),
                },
            ],
            body: "OK".into(),
        });
    }

    #[test]
    fn http_response_empty_round_trip() {
        round_trip(&HttpResponse {
            status: 204,
            headers: vec![],
            body: String::new(),
        });
    }
}
