//! LLM types for messages, tools, and streaming.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Message role.
    pub role: MessageRole,
    /// Message content.
    pub content: MessageContent,
}

impl Message {
    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: MessageContent::Text(content.into()),
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: MessageContent::Text(content.into()),
        }
    }

    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: MessageContent::Text(content.into()),
        }
    }

    /// Create an assistant message with tool calls.
    #[must_use]
    pub fn assistant_with_tools(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: MessageContent::ToolCalls(tool_calls),
        }
    }

    /// Create a tool result message.
    #[must_use]
    pub fn tool_result(result: ToolCallResult) -> Self {
        Self {
            role: MessageRole::Tool,
            content: MessageContent::ToolResult(result),
        }
    }

    /// Get text content if this is a text message.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match &self.content {
            MessageContent::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Get tool calls if this is a tool call message.
    #[must_use]
    pub fn tool_calls(&self) -> Option<&[ToolCall]> {
        match &self.content {
            MessageContent::ToolCalls(calls) => Some(calls),
            _ => None,
        }
    }
}

/// Message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// System message (instructions).
    System,
    /// User message.
    User,
    /// Assistant message.
    Assistant,
    /// Tool result.
    Tool,
}

/// Message content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text content.
    Text(String),
    /// Tool calls.
    ToolCalls(Vec<ToolCall>),
    /// Tool result.
    ToolResult(ToolCallResult),
    /// Multi-part content (text + images).
    MultiPart(Vec<ContentPart>),
}

/// A part of multi-part content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text content.
    Text {
        /// The text.
        text: String,
    },
    /// Image content.
    Image {
        /// Base64-encoded image data.
        data: String,
        /// MIME type.
        media_type: String,
    },
}

/// A tool call from the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique call ID.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Tool arguments (JSON).
    pub arguments: Value,
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: Value::Object(serde_json::Map::new()),
        }
    }

    /// Set arguments.
    #[must_use]
    pub fn with_arguments(mut self, args: Value) -> Self {
        self.arguments = args;
        self
    }

    /// Parse the server and tool name from "server:tool" format.
    #[must_use]
    pub fn parse_name(&self) -> Option<(&str, &str)> {
        self.name.split_once(':')
    }
}

/// Result of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// Tool call ID this is responding to.
    pub call_id: String,
    /// Result content.
    pub content: String,
    /// Whether this is an error result.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolCallResult {
    /// Create a successful result.
    pub fn success(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            call_id: call_id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    /// Create an error result.
    pub fn error(call_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            call_id: call_id.into(),
            content: error.into(),
            is_error: true,
        }
    }
}

/// Tool definition for the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolDefinition {
    /// Tool name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Input JSON schema.
    pub input_schema: Value,
}

impl LlmToolDefinition {
    /// Create a new tool definition.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    /// Set description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set input schema.
    #[must_use]
    pub fn with_schema(mut self, schema: Value) -> Self {
        self.input_schema = schema;
        self
    }
}

/// Streaming event from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    /// Partial text output.
    TextDelta(String),
    /// Tool call started.
    ToolCallStart {
        /// Call ID.
        id: String,
        /// Tool name.
        name: String,
    },
    /// Tool call arguments delta.
    ToolCallDelta {
        /// Call ID.
        id: String,
        /// Partial arguments JSON.
        args_delta: String,
    },
    /// Tool call completed.
    ToolCallEnd {
        /// Call ID.
        id: String,
    },
    /// Reasoning/chain-of-thought delta (used by Z.AI, `DeepSeek`, `OpenAI` o-series, etc.).
    ReasoningDelta(String),
    /// Usage information.
    Usage {
        /// Input tokens.
        input_tokens: usize,
        /// Output tokens.
        output_tokens: usize,
    },
    /// Stream completed.
    Done,
    /// Error occurred.
    Error(String),
}

/// LLM response (non-streaming).
#[derive(Debug, Clone)]
pub struct LlmResponse {
    /// Response message.
    pub message: Message,
    /// Whether the response has tool calls.
    pub has_tool_calls: bool,
    /// Stop reason.
    pub stop_reason: StopReason,
    /// Token usage.
    pub usage: Usage,
}

/// Reason the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Natural end of response.
    EndTurn,
    /// Hit max tokens.
    MaxTokens,
    /// Tool use requested.
    ToolUse,
    /// Stop sequence hit.
    StopSequence,
}

/// Token usage information.
#[derive(Debug, Clone, Default)]
pub struct Usage {
    /// Input tokens.
    pub input_tokens: usize,
    /// Output tokens.
    pub output_tokens: usize,
}

impl Usage {
    /// Total tokens.
    #[must_use]
    pub fn total(&self) -> usize {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let user = Message::user("Hello");
        assert_eq!(user.role, MessageRole::User);
        assert_eq!(user.text(), Some("Hello"));

        let assistant = Message::assistant("Hi there!");
        assert_eq!(assistant.role, MessageRole::Assistant);
    }

    #[test]
    fn test_tool_call() {
        let call = ToolCall::new("123", "filesystem:read_file")
            .with_arguments(serde_json::json!({"path": "/tmp/test.txt"}));

        assert_eq!(call.parse_name(), Some(("filesystem", "read_file")));
    }

    #[test]
    fn test_tool_result() {
        let success = ToolCallResult::success("123", "file contents");
        assert!(!success.is_error);

        let error = ToolCallResult::error("123", "file not found");
        assert!(error.is_error);
    }
}
