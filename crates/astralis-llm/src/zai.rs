//! Z.AI (Zhipu AI) LLM provider implementation.
//!
//! Z.AI offers OpenAI-compatible chat completion APIs with unique features:
//! - `reasoning_content` in streaming deltas for chain-of-thought tokens
//! - Z.AI-specific `finish_reason` values (`sensitive`, `network_error`)
//!
//! Uses the `OpenAI` chat completion format but with Z.AI-specific extensions.

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

const ZAI_API_URL: &str = "https://api.z.ai/api/paas/v4/chat/completions";

/// Z.AI (Zhipu AI) LLM provider.
///
/// Supports GLM-4 series models with OpenAI-compatible format and
/// reasoning content streaming.
pub struct ZaiProvider {
    client: Client,
    config: ProviderConfig,
    max_context: usize,
}

impl ZaiProvider {
    /// Create a new Z.AI provider.
    #[must_use]
    pub fn new(config: ProviderConfig) -> Self {
        let max_context = match config.model.as_str() {
            m if m.starts_with("glm-4.7") => 200_000,
            m if m.starts_with("glm-4.6") => 200_000,
            m if m.starts_with("glm-4.5") => 128_000,
            m if m.starts_with("glm-4-32b") => 128_000,
            _ => 128_000,
        };

        Self {
            client: Client::new(),
            config,
            max_context,
        }
    }

    /// Build the request body.
    fn build_request(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
        stream: bool,
    ) -> Value {
        let mut zai_messages = Vec::new();

        // Add system message.
        if !system.is_empty() {
            zai_messages.push(serde_json::json!({
                "role": "system",
                "content": system
            }));
        }

        // Convert messages.
        for msg in messages {
            zai_messages.push(convert_message(msg));
        }

        let mut request = serde_json::json!({
            "model": self.config.model,
            "messages": zai_messages,
            "max_tokens": self.config.max_tokens,
            "temperature": self.config.temperature,
            "stream": stream
        });

        // Add tools if provided.
        if !tools.is_empty() {
            let zai_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema
                        }
                    })
                })
                .collect();
            request["tools"] = Value::Array(zai_tools);
        }

        request
    }
}

fn convert_message(msg: &Message) -> Value {
    let role = match msg.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    };

    match &msg.content {
        MessageContent::Text(text) => {
            serde_json::json!({
                "role": role,
                "content": text
            })
        },
        MessageContent::ToolCalls(tool_calls) => {
            let zai_tool_calls: Vec<Value> = tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default()
                        }
                    })
                })
                .collect();

            serde_json::json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": zai_tool_calls
            })
        },
        MessageContent::ToolResult(result) => {
            serde_json::json!({
                "role": "tool",
                "tool_call_id": result.call_id,
                "content": result.content
            })
        },
        MessageContent::MultiPart(parts) => {
            let content: Vec<Value> = parts
                .iter()
                .map(|part| match part {
                    crate::types::ContentPart::Text { text } => {
                        serde_json::json!({
                            "type": "text",
                            "text": text
                        })
                    },
                    crate::types::ContentPart::Image { data, media_type } => {
                        serde_json::json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{media_type};base64,{data}")
                            }
                        })
                    },
                })
                .collect();

            serde_json::json!({
                "role": role,
                "content": content
            })
        },
    }
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl LlmProvider for ZaiProvider {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "Z.AI"
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn max_context_length(&self) -> usize {
        self.max_context
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<StreamBox> {
        if self.config.api_key.is_empty() {
            return Err(LlmError::ApiKeyNotConfigured {
                provider: "zai".to_string(),
            });
        }

        let request_body = self.build_request(messages, tools, system, true);
        let url = self.config.base_url.as_deref().unwrap_or(ZAI_API_URL);

        debug!(model = %self.config.model, url = %url, "Starting Z.AI stream");

        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| LlmError::ApiRequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!(status = %status, body = %body, "Z.AI API error");
            let status_code = status.as_u16();
            return Err(LlmError::InvalidResponse(format!(
                "HTTP {status_code}: {body}"
            )));
        }

        #[allow(clippy::collapsible_if)]
        let stream = try_stream! {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut current_tool_call: Option<PartialToolCall> = None;

            use futures::StreamExt;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| LlmError::StreamingError(e.to_string()))?;
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                // Process complete SSE events.
                while let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data.trim() == "[DONE]" {
                                if let Some(tc) = current_tool_call.take() {
                                    yield StreamEvent::ToolCallEnd { id: tc.id };
                                }
                                yield StreamEvent::Done;
                                return;
                            }

                            if let Ok(event) = serde_json::from_str::<ZaiStreamEvent>(data) {
                                if let Some(choice) = event.choices.first() {
                                    // Handle reasoning content delta.
                                    if let Some(reasoning) = &choice.delta.reasoning_content {
                                        if !reasoning.is_empty() {
                                            yield StreamEvent::ReasoningDelta(reasoning.clone());
                                        }
                                    }

                                    // Handle content delta.
                                    if let Some(content) = &choice.delta.content {
                                        if !content.is_empty() {
                                            yield StreamEvent::TextDelta(content.clone());
                                        }
                                    }

                                    // Handle tool calls.
                                    if let Some(tool_calls) = &choice.delta.tool_calls {
                                        for tc in tool_calls {
                                            if let Some(function) = &tc.function {
                                                // Check if this is a new tool call.
                                                if tc.id.is_some() || current_tool_call.is_none() {
                                                    // End previous tool call if exists.
                                                    if let Some(prev) = current_tool_call.take() {
                                                        yield StreamEvent::ToolCallEnd { id: prev.id };
                                                    }

                                                    // Start new tool call.
                                                    let id = tc.id.clone().unwrap_or_else(|| format!("call_{index}", index = tc.index));
                                                    let name = function.name.clone().unwrap_or_default();

                                                    yield StreamEvent::ToolCallStart {
                                                        id: id.clone(),
                                                        name: name.clone(),
                                                    };

                                                    current_tool_call = Some(PartialToolCall {
                                                        id,
                                                        name,
                                                        arguments: String::new(),
                                                    });
                                                }

                                                // Append arguments.
                                                if let Some(args) = &function.arguments {
                                                    if let Some(ref mut tc) = current_tool_call {
                                                        tc.arguments.push_str(args);
                                                        yield StreamEvent::ToolCallDelta {
                                                            id: tc.id.clone(),
                                                            args_delta: args.clone(),
                                                        };
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Handle finish reason.
                                    if let Some(ref reason) = choice.finish_reason {
                                        if let Some(tc) = current_tool_call.take() {
                                            yield StreamEvent::ToolCallEnd { id: tc.id };
                                        }

                                        // Send usage if available.
                                        if let Some(usage) = &event.usage {
                                            yield StreamEvent::Usage {
                                                input_tokens: usage.prompt_tokens,
                                                output_tokens: usage.completion_tokens,
                                            };
                                        }

                                        if reason == "stop" || reason == "tool_calls" || reason == "sensitive" || reason == "network_error" {
                                            yield StreamEvent::Done;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Handle any remaining tool call.
            if let Some(tc) = current_tool_call.take() {
                yield StreamEvent::ToolCallEnd { id: tc.id };
            }
            yield StreamEvent::Done;
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
                provider: "zai".to_string(),
            });
        }

        let request_body = self.build_request(messages, tools, system, false);
        let url = self.config.base_url.as_deref().unwrap_or(ZAI_API_URL);

        debug!(model = %self.config.model, url = %url, "Making Z.AI completion request");

        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| LlmError::ApiRequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let status_code = status.as_u16();
            return Err(LlmError::InvalidResponse(format!(
                "HTTP {status_code}: {body}"
            )));
        }

        let response: ZaiResponse = response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let choice = response
            .choices
            .first()
            .ok_or_else(|| LlmError::InvalidResponse("No choices in response".to_string()))?;

        // Build message content and check for tool calls.
        let (content, has_tool_calls) = match &choice.message.tool_calls {
            Some(tool_calls) if !tool_calls.is_empty() => {
                let calls: Vec<ToolCall> = tool_calls
                    .iter()
                    .map(|tc| {
                        let arguments: Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(Value::Object(serde_json::Map::new()));
                        ToolCall {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            arguments,
                        }
                    })
                    .collect();
                (MessageContent::ToolCalls(calls), true)
            },
            _ => (
                MessageContent::Text(choice.message.content.clone().unwrap_or_default()),
                false,
            ),
        };

        let message = Message {
            role: MessageRole::Assistant,
            content,
        };

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("length") => StopReason::MaxTokens,
            Some("tool_calls") => StopReason::ToolUse,
            Some("sensitive" | "network_error") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        Ok(LlmResponse {
            message,
            has_tool_calls,
            stop_reason,
            usage: Usage {
                input_tokens: response.usage.prompt_tokens,
                output_tokens: response.usage.completion_tokens,
            },
        })
    }
}

impl std::fmt::Debug for ZaiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZaiProvider")
            .field("model", &self.config.model)
            .field("max_context", &self.max_context)
            .finish_non_exhaustive()
    }
}

// Helper struct for tracking partial tool calls during streaming.
struct PartialToolCall {
    id: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    arguments: String,
}

// Z.AI API response types (OpenAI-compatible).

#[derive(Debug, Deserialize)]
struct ZaiResponse {
    choices: Vec<ZaiChoice>,
    usage: ZaiUsage,
}

#[derive(Debug, Deserialize)]
struct ZaiChoice {
    message: ZaiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZaiMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ZaiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ZaiToolCall {
    id: String,
    function: ZaiFunctionCall,
}

#[derive(Debug, Deserialize)]
struct ZaiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ZaiUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

// Streaming response types.

#[derive(Debug, Deserialize)]
struct ZaiStreamEvent {
    choices: Vec<ZaiStreamChoice>,
    usage: Option<ZaiUsage>,
}

#[derive(Debug, Deserialize)]
struct ZaiStreamChoice {
    delta: ZaiDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZaiDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<ZaiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ZaiStreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<ZaiStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct ZaiStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCallResult;

    #[test]
    fn test_provider_creation() {
        let config = ProviderConfig::new("test-key", "glm-4.7-plus");
        let provider = ZaiProvider::new(config);
        assert_eq!(provider.name(), "Z.AI");
        assert_eq!(provider.model(), "glm-4.7-plus");
        assert_eq!(provider.max_context_length(), 200_000);
    }

    #[test]
    fn test_context_length_by_model() {
        let cases = vec![
            ("glm-4.7-plus", 200_000),
            ("glm-4.6-flash", 200_000),
            ("glm-4.5-chat", 128_000),
            ("glm-4-32b-instruct", 128_000),
            ("unknown-model", 128_000),
        ];

        for (model, expected) in cases {
            let config = ProviderConfig::new("key", model);
            let provider = ZaiProvider::new(config);
            assert_eq!(
                provider.max_context_length(),
                expected,
                "model {model} should have context length {expected}"
            );
        }
    }

    #[test]
    fn test_message_conversion() {
        let msg = Message::user("Hello");
        let converted = convert_message(&msg);
        assert_eq!(converted["role"], "user");
        assert_eq!(converted["content"], "Hello");
    }

    #[test]
    fn test_tool_result_conversion() {
        let result = ToolCallResult::success("call_123", "File contents here");
        let msg = Message::tool_result(result);
        let converted = convert_message(&msg);

        assert_eq!(converted["role"], "tool");
        assert_eq!(converted["tool_call_id"], "call_123");
        assert_eq!(converted["content"], "File contents here");
    }

    #[test]
    fn test_build_request() {
        let config = ProviderConfig::new("test-key", "glm-4.7-plus");
        let provider = ZaiProvider::new(config);
        let messages = vec![Message::user("Hi")];
        let request = provider.build_request(&messages, &[], "Be helpful", false);

        assert_eq!(request["model"], "glm-4.7-plus");
        assert_eq!(request["stream"], false);
        assert!(request["messages"].as_array().unwrap().len() >= 2); // system + user
    }

    #[test]
    fn test_build_request_with_tools() {
        let config = ProviderConfig::new("test-key", "glm-4.7-plus");
        let provider = ZaiProvider::new(config);
        let messages = vec![Message::user("Read a file")];
        let tools =
            vec![LlmToolDefinition::new("read_file").with_description("Read a file from disk")];
        let request = provider.build_request(&messages, &tools, "", false);

        let tools_json = request["tools"].as_array().unwrap();
        assert_eq!(tools_json.len(), 1);
        assert_eq!(tools_json[0]["type"], "function");
        assert_eq!(tools_json[0]["function"]["name"], "read_file");
    }
}
