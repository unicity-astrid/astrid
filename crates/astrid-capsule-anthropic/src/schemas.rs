//! Anthropic-specific SSE stream parsing types.
//!
//! These types map directly to the Anthropic Messages API streaming format.
//! They are deserialized from SSE `data:` lines and converted to the
//! standardized `astrid_events::llm::StreamEvent` variants.

use serde::Deserialize;
use serde_json::Value;

/// Top-level SSE event from the Anthropic Messages API.
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamingEvent {
    /// Sent at the start of a new message.
    MessageStart {
        /// The partial message object.
        message: Value,
    },
    /// Sent when a new content block begins (text or tool_use).
    ContentBlockStart {
        /// Block index within the message.
        index: usize,
        /// The content block definition.
        content_block: ContentBlock,
    },
    /// Sent for incremental content within a block.
    ContentBlockDelta {
        /// Block index within the message.
        index: usize,
        /// The incremental delta.
        delta: Delta,
    },
    /// Sent when a content block finishes.
    ContentBlockStop {
        /// Block index within the message.
        index: usize,
    },
    /// Sent when message-level metadata changes (e.g. final usage).
    MessageDelta {
        /// Delta metadata.
        delta: Value,
        /// Optional usage statistics.
        usage: Option<AnthropicUsage>,
    },
    /// Sent when the entire message is complete.
    MessageStop,
    /// Keep-alive ping.
    Ping,
    /// API error during streaming.
    Error {
        /// Error details.
        error: Value,
    },
}

/// A content block in the Anthropic streaming response.
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// A text content block.
    Text {
        /// Initial text content (usually empty for streaming).
        text: String,
    },
    /// A tool use content block.
    ToolUse {
        /// Unique tool call ID.
        id: String,
        /// Tool name.
        name: String,
        /// Partial input (usually empty object for streaming).
        input: Value,
    },
}

/// Incremental delta within a content block.
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delta {
    /// Incremental text.
    TextDelta {
        /// The text fragment.
        text: String,
    },
    /// Incremental JSON for tool input.
    InputJsonDelta {
        /// Partial JSON string to append.
        partial_json: String,
    },
}

/// Token usage statistics from the Anthropic API.
#[derive(Deserialize, Debug)]
pub struct AnthropicUsage {
    /// Input tokens consumed (present on message_delta).
    pub input_tokens: Option<usize>,
    /// Output tokens generated.
    pub output_tokens: usize,
}

/// HTTP request payload for the SDK http airlock.
#[derive(serde::Serialize)]
pub struct HttpRequest {
    /// Target URL.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Request headers.
    pub headers: std::collections::HashMap<String, String>,
    /// Optional request body.
    pub body: Option<String>,
}

/// HTTP response payload from the SDK http airlock.
#[derive(Deserialize)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    #[allow(dead_code)]
    pub headers: std::collections::HashMap<String, String>,
    /// Response body.
    pub body: String,
}
