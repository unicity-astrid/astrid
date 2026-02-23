//! Claude (Anthropic) LLM provider implementation.

use async_stream::try_stream;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, error};

use crate::error::{LlmError, LlmResult};
use crate::provider::{LlmProvider, ProviderConfig, StreamBox};
use crate::types::{
    LlmResponse, LlmToolDefinition, Message, MessageContent, MessageRole, StopReason, StreamEvent,
    ToolCall, Usage,
};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Claude LLM provider.
pub struct ClaudeProvider {
    client: Client,
    config: ProviderConfig,
}

impl ClaudeProvider {
    /// Create a new Claude provider.
    #[must_use]
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    /// Build the API request body.
    fn build_request(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
        stream: bool,
    ) -> Value {
        let api_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(Self::convert_message)
            .collect();

        let mut request = serde_json::json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "messages": api_messages,
            "stream": stream,
        });

        if !system.is_empty() {
            request["system"] = Value::String(system.to_string());
        }

        if !tools.is_empty() {
            let api_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect();
            request["tools"] = Value::Array(api_tools);
        }

        request
    }

    /// Convert our Message to Anthropic format.
    fn convert_message(message: &Message) -> Value {
        match &message.content {
            MessageContent::Text(text) => {
                serde_json::json!({
                    "role": match message.role {
                        MessageRole::Assistant => "assistant",
                        MessageRole::User | MessageRole::Tool | MessageRole::System => "user",
                    },
                    "content": text,
                })
            },
            MessageContent::ToolCalls(calls) => {
                let content: Vec<Value> = calls
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.name,
                            "input": c.arguments,
                        })
                    })
                    .collect();

                serde_json::json!({
                    "role": "assistant",
                    "content": content,
                })
            },
            MessageContent::ToolResult(result) => {
                serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": result.call_id,
                        "content": result.content,
                        "is_error": result.is_error,
                    }],
                })
            },
            MessageContent::MultiPart(parts) => {
                let content: Vec<Value> = parts
                    .iter()
                    .map(|p| match p {
                        crate::types::ContentPart::Text { text } => {
                            serde_json::json!({"type": "text", "text": text})
                        },
                        crate::types::ContentPart::Image { data, media_type } => {
                            serde_json::json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": data,
                                }
                            })
                        },
                    })
                    .collect();

                serde_json::json!({
                    "role": match message.role {
                        MessageRole::Assistant => "assistant",
                        MessageRole::User | MessageRole::Tool | MessageRole::System => "user",
                    },
                    "content": content,
                })
            },
        }
    }

    /// Parse a response into our types.
    fn parse_response(response: &ApiResponse) -> LlmResponse {
        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        for block in &response.content {
            match block {
                ContentBlock::Text { text } => {
                    text_content.push_str(text);
                },
                ContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: input.clone(),
                    });
                },
            }
        }

        let message = if tool_calls.is_empty() {
            Message::assistant(text_content)
        } else {
            Message::assistant_with_tools(tool_calls)
        };

        let stop_reason = match response.stop_reason.as_deref() {
            Some("max_tokens") => StopReason::MaxTokens,
            Some("tool_use") => StopReason::ToolUse,
            Some("stop_sequence") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        LlmResponse {
            has_tool_calls: matches!(stop_reason, StopReason::ToolUse),
            message,
            stop_reason,
            usage: Usage {
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
            },
        }
    }
}

#[async_trait]
impl LlmProvider for ClaudeProvider {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "Anthropic Claude"
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    #[allow(clippy::too_many_lines)]
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<StreamBox> {
        if self.config.api_key.is_empty() {
            return Err(LlmError::ApiKeyNotConfigured {
                provider: "claude".to_string(),
            });
        }

        let request_body = self.build_request(messages, tools, system, true);
        let url = self.config.base_url.as_deref().unwrap_or(ANTHROPIC_API_URL);

        debug!(model = self.config.model, "Starting Claude stream");

        let mut api_key_header = reqwest::header::HeaderValue::try_from(&self.config.api_key)
            .map_err(|e| LlmError::ConfigError(format!("Invalid API key characters: {e}")))?;
        api_key_header.set_sensitive(true);

        let response = self
            .client
            .post(url)
            .header("x-api-key", api_key_header)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!(status = %status, body = %body, "Claude API error");

            if status.as_u16() == 429 {
                return Err(LlmError::RateLimitExceeded {
                    retry_after_secs: 60,
                });
            }

            return Err(LlmError::ApiRequestFailed(format!(
                "Status {status}: {body}"
            )));
        }

        // Create a stream that parses SSE events
        let stream = try_stream! {
            let mut bytes_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut current_tool_id = String::new();

            use futures::StreamExt;

            while let Some(chunk) = bytes_stream.next().await {
                let chunk = chunk?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE events
                while let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    // Safety: event_end from find() is within bounds, +2 for "\n\n" length
                    #[allow(clippy::arithmetic_side_effects)]
                    let rest_start = event_end + 2;
                    buffer = buffer[rest_start..].to_string();

                    // Parse SSE event
                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                yield StreamEvent::Done;
                                continue;
                            }

                            if let Ok(event) = serde_json::from_str::<StreamingEvent>(data) {
                                match event {
                                    StreamingEvent::ContentBlockStart { index: _, content_block } => {
                                        match content_block {
                                            ContentBlock::Text { .. } => {}
                                            ContentBlock::ToolUse { id, name, .. } => {
                                                current_tool_id = id.clone();
                                                yield StreamEvent::ToolCallStart { id, name };
                                            }
                                        }
                                    }
                                    StreamingEvent::ContentBlockDelta { delta, .. } => {
                                        match delta {
                                            Delta::TextDelta { text } => {
                                                yield StreamEvent::TextDelta(text);
                                            }
                                            Delta::InputJsonDelta { partial_json } => {
                                                yield StreamEvent::ToolCallDelta {
                                                    id: current_tool_id.clone(),
                                                    args_delta: partial_json,
                                                };
                                            }
                                        }
                                    }
                                    StreamingEvent::ContentBlockStop { .. } => {
                                        if !current_tool_id.is_empty() {
                                            yield StreamEvent::ToolCallEnd {
                                                id: current_tool_id.clone(),
                                            };
                                            current_tool_id.clear();
                                        }
                                    }
                                    StreamingEvent::MessageDelta { usage: Some(usage), .. } => {
                                        yield StreamEvent::Usage {
                                            input_tokens: 0, // Not provided in delta
                                            output_tokens: usage.output_tokens,
                                        };
                                    }
                                    StreamingEvent::MessageStop => {
                                        yield StreamEvent::Done;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<LlmResponse> {
        if self.config.api_key.is_empty() {
            return Err(LlmError::ApiKeyNotConfigured {
                provider: "claude".to_string(),
            });
        }

        let request_body = self.build_request(messages, tools, system, false);
        let url = self.config.base_url.as_deref().unwrap_or(ANTHROPIC_API_URL);

        debug!(model = self.config.model, "Sending Claude request");

        let mut api_key_header = reqwest::header::HeaderValue::try_from(&self.config.api_key)
            .map_err(|e| LlmError::ConfigError(format!("Invalid API key characters: {e}")))?;
        api_key_header.set_sensitive(true);

        let response = self
            .client
            .post(url)
            .header("x-api-key", api_key_header)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!(status = %status, body = %body, "Claude API error");

            if status.as_u16() == 429 {
                return Err(LlmError::RateLimitExceeded {
                    retry_after_secs: 60,
                });
            }

            return Err(LlmError::ApiRequestFailed(format!(
                "Status {status}: {body}"
            )));
        }

        let api_response: ApiResponse = response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        Ok(Self::parse_response(&api_response))
    }

    fn max_context_length(&self) -> usize {
        // Claude 3.5 Sonnet has 200k context
        200_000
    }
}

// API response types

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    input_tokens: usize,
    output_tokens: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

// Streaming event types

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)] // Fields required for deserialization
enum StreamingEvent {
    MessageStart {
        message: Value,
    },
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: Value,
        usage: Option<DeltaUsage>,
    },
    MessageStop,
    Ping,
    Error {
        error: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Delta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize)]
struct DeltaUsage {
    output_tokens: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_invalid_api_key_characters() {
        let config = ProviderConfig::new("invalid\nkey", "claude-3-sonnet");
        let provider = ClaudeProvider::new(config);
        let Err(err_complete) = provider.complete(&[], &[], "").await else {
            panic!("Expected error");
        };
        assert!(
            matches!(err_complete, LlmError::ConfigError(ref msg) if msg.contains("Invalid API key characters"))
        );

        let Err(err_stream) = provider.stream(&[], &[], "").await else {
            panic!("Expected error");
        };
        assert!(
            matches!(err_stream, LlmError::ConfigError(ref msg) if msg.contains("Invalid API key characters"))
        );
    }

    #[test]
    fn test_build_request() {
        let config = ProviderConfig::new("test-key", "claude-3-sonnet");
        let provider = ClaudeProvider::new(config);

        let messages = vec![Message::user("Hello")];
        let request = provider.build_request(&messages, &[], "You are helpful", false);

        assert_eq!(request["model"], "claude-3-sonnet");
        assert_eq!(request["system"], "You are helpful");
        assert!(!request["stream"].as_bool().unwrap());
    }

    #[test]
    fn test_convert_message() {
        let msg = Message::user("Hello");
        let converted = ClaudeProvider::convert_message(&msg);

        assert_eq!(converted["role"], "user");
        assert_eq!(converted["content"], "Hello");
    }
}
