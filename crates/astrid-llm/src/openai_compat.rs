//! OpenAI-compatible LLM provider implementation.
//!
//! Works with:
//! - LM Studio (localhost:1234)
//! - `OpenAI` API
//! - vLLM
//! - Ollama (with `OpenAI` compatibility)
//! - Any `OpenAI`-compatible endpoint

use async_stream::try_stream;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, error};

use crate::error::{LlmError, LlmResult};
use crate::provider::{LlmProvider, StreamBox};
use crate::types::{
    LlmResponse, LlmToolDefinition, Message, MessageContent, MessageRole, StopReason, StreamEvent,
    ToolCall, Usage,
};

const DEFAULT_LM_STUDIO_URL: &str = "http://localhost:1234/v1/chat/completions";
const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

/// OpenAI-compatible LLM provider.
///
/// Works with LM Studio, `OpenAI`, and other compatible APIs.
pub struct OpenAiCompatProvider {
    client: Client,
    model: String,
    max_tokens: usize,
    temperature: f64,
    base_url: String,
    api_key: Option<String>,
    max_context: usize,
}

impl OpenAiCompatProvider {
    /// Create a new provider for LM Studio (localhost:1234).
    #[must_use]
    pub fn lm_studio() -> Self {
        Self::lm_studio_with_model("local-model")
    }

    /// Create a new provider for LM Studio with a specific model name.
    #[must_use]
    pub fn lm_studio_with_model(model: &str) -> Self {
        Self {
            client: Client::new(),
            model: model.to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            base_url: DEFAULT_LM_STUDIO_URL.to_string(),
            api_key: None,      // LM Studio doesn't require auth by default
            max_context: 32768, // Reasonable default for local models
        }
    }

    /// Create a new provider for `OpenAI`.
    #[must_use]
    pub fn openai(api_key: &str, model: &str) -> Self {
        let max_context = match model {
            m if m.contains("gpt-4o") => 128_000,
            m if m.contains("gpt-4-turbo") => 128_000,
            m if m.contains("gpt-4-32k") => 32_768,
            m if m.contains("gpt-4") => 8_192,
            m if m.contains("gpt-3.5-turbo-16k") => 16_385,
            m if m.contains("gpt-3.5-turbo") => 16_385,
            _ => 8_192,
        };

        Self {
            client: Client::new(),
            model: model.to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            base_url: DEFAULT_OPENAI_URL.to_string(),
            api_key: Some(api_key.to_string()),
            max_context,
        }
    }

    /// Create a custom provider with full configuration.
    #[must_use]
    pub fn custom(base_url: &str, api_key: Option<&str>, model: &str) -> Self {
        Self {
            client: Client::new(),
            model: model.to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            base_url: base_url.to_string(),
            api_key: api_key.map(ToString::to_string),
            max_context: 32768,
        }
    }

    /// Set max tokens.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Set temperature.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = temperature;
        self
    }

    /// Set maximum context length.
    #[must_use]
    pub fn with_max_context(mut self, max_context: usize) -> Self {
        self.max_context = max_context;
        self
    }

    /// Build the request body.
    fn build_request(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
        stream: bool,
    ) -> Value {
        let mut openai_messages = Vec::new();

        // Add system message
        if !system.is_empty() {
            openai_messages.push(serde_json::json!({
                "role": "system",
                "content": system
            }));
        }

        // Convert messages
        for msg in messages {
            openai_messages.push(convert_message(msg));
        }

        let mut request = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "stream": stream
        });

        // Add tools if provided
        if !tools.is_empty() {
            let openai_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    // Ensure `properties` is always an object (even if empty).
                    // Strict OpenAI-compatible endpoints (e.g. LM Studio) use Zod
                    // validation that rejects a missing `properties` field with HTTP 400.
                    let mut parameters = t.input_schema.clone();
                    if let Some(obj) = parameters.as_object_mut() {
                        obj.entry("properties")
                            .or_insert_with(|| serde_json::json!({}));
                    }
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": parameters
                        }
                    })
                })
                .collect();
            request["tools"] = Value::Array(openai_tools);
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
            // Assistant message with tool calls
            let openai_tool_calls: Vec<Value> = tool_calls
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
                "tool_calls": openai_tool_calls
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
            // Convert multi-part content to OpenAI format
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
impl LlmProvider for OpenAiCompatProvider {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "openai-compat"
    }

    fn model(&self) -> &str {
        &self.model
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
        // Remote endpoints require an API key; local endpoints (LM Studio,
        // Ollama, vLLM) typically do not.
        if self.api_key.as_ref().is_none_or(String::is_empty) && !is_local_url(&self.base_url) {
            return Err(LlmError::ApiKeyNotConfigured {
                provider: "openai-compat".to_string(),
            });
        }

        let request_body = self.build_request(messages, tools, system, true);

        debug!(
            model = %self.model,
            base_url = %self.base_url,
            "Starting OpenAI-compatible stream"
        );

        let mut request = self
            .client
            .post(&self.base_url)
            .header("Content-Type", "application/json");

        // Add auth header if API key is present
        if let Some(ref api_key) = self.api_key {
            let mut auth_value = reqwest::header::HeaderValue::try_from(format!(
                "Bearer {api_key}"
            ))
            .map_err(|e| LlmError::ApiRequestFailed(format!("Invalid API key characters: {e}")))?;
            auth_value.set_sensitive(true);
            request = request.header("Authorization", auth_value);
        }

        let response = request
            .json(&request_body)
            .send()
            .await
            .map_err(|e| LlmError::ApiRequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!(status = %status, body = %body, "OpenAI API error");
            let status_code = status.as_u16();
            return Err(LlmError::InvalidResponse(format!(
                "HTTP {status_code}: {body}"
            )));
        }

        #[allow(clippy::collapsible_if)]
        let stream = try_stream! {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut current_tool_call: Option<String> = None;

            use futures::StreamExt;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| LlmError::StreamingError(e.to_string()))?;
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                // Process complete SSE events
                while let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    // Safety: event_end from find() is within bounds, +2 for "\n\n" length
                    #[allow(clippy::arithmetic_side_effects)]
                    let rest_start = event_end + 2;
                    buffer = buffer[rest_start..].to_string();

                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data.trim() == "[DONE]" {
                                                                 if let Some(tc_id) = current_tool_call.take() {
                                                                     yield StreamEvent::ToolCallEnd { id: tc_id };
                                                                 }                                yield StreamEvent::Done;
                                return;
                            }

                            if let Ok(event) = serde_json::from_str::<OpenAiStreamEvent>(data) {
                                if let Some(choice) = event.choices.first() {
                                    // Handle content delta
                                    if let Some(content) = &choice.delta.content {
                                        if !content.is_empty() {
                                            yield StreamEvent::TextDelta(content.clone());
                                        }
                                    }

                                    // Handle tool calls
                                    if let Some(tool_calls) = &choice.delta.tool_calls {
                                        for tc in tool_calls {
                                            if let Some(function) = &tc.function {
                                                // Check if this is a new tool call
                                                if tc.id.is_some() || current_tool_call.is_none() {
                                                    // End previous tool call if exists
                                                    if let Some(prev_id) = current_tool_call.take() {
                                                        yield StreamEvent::ToolCallEnd { id: prev_id };
                                                    }

                                                    // Start new tool call
                                                    let id = tc.id.clone().unwrap_or_else(|| format!("call_{index}", index = tc.index));
                                                    let name = function.name.clone().unwrap_or_default();

                                                    yield StreamEvent::ToolCallStart {
                                                        id: id.clone(),
                                                        name: name.clone(),
                                                    };

                                                    current_tool_call = Some(id);
                                                }

                                                // Append arguments
                                                if let Some(args) = &function.arguments {
                                                    if let Some(ref tc_id) = current_tool_call {
                                                        yield StreamEvent::ToolCallDelta {
                                                            id: tc_id.clone(),
                                                            args_delta: args.clone(),
                                                        };
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Handle finish reason
                                    if let Some(ref reason) = choice.finish_reason {
                                        if let Some(tc_id) = current_tool_call.take() {
                                            yield StreamEvent::ToolCallEnd { id: tc_id };
                                        }

                                        // Send usage if available
                                        if let Some(usage) = &event.usage {
                                            yield StreamEvent::Usage {
                                                input_tokens: usage.prompt_tokens,
                                                output_tokens: usage.completion_tokens,
                                            };
                                        }

                                        if reason == "stop" || reason == "tool_calls" {
                                            yield StreamEvent::Done;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Handle any remaining tool call
                                             if let Some(tc_id) = current_tool_call.take() {
                                                 yield StreamEvent::ToolCallEnd { id: tc_id };
                                             }            yield StreamEvent::Done;
        };

        Ok(Box::pin(stream))
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<LlmResponse> {
        if self.api_key.as_ref().is_none_or(String::is_empty) && !is_local_url(&self.base_url) {
            return Err(LlmError::ApiKeyNotConfigured {
                provider: "openai-compat".to_string(),
            });
        }

        let request_body = self.build_request(messages, tools, system, false);

        debug!(
            model = %self.model,
            base_url = %self.base_url,
            "Making OpenAI-compatible completion request"
        );

        let mut request = self
            .client
            .post(&self.base_url)
            .header("Content-Type", "application/json");

        if let Some(ref api_key) = self.api_key {
            let mut auth_value = reqwest::header::HeaderValue::try_from(format!(
                "Bearer {api_key}"
            ))
            .map_err(|e| LlmError::ApiRequestFailed(format!("Invalid API key characters: {e}")))?;
            auth_value.set_sensitive(true);
            request = request.header("Authorization", auth_value);
        }

        let response = request
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

        let response: OpenAiResponse = response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        // Convert to our response type
        let choice = response
            .choices
            .first()
            .ok_or_else(|| LlmError::InvalidResponse("No choices in response".to_string()))?;

        // Build message content and check for tool calls
        let (content, has_tool_calls) = match &choice.message.tool_calls {
            Some(tool_calls) if !tool_calls.is_empty() => {
                let mut calls = Vec::new();
                for tc in tool_calls {
                    let arguments: Value =
                        serde_json::from_str(&tc.function.arguments).map_err(|e| {
                            LlmError::InvalidResponse(format!("Invalid tool arguments JSON: {e}"))
                        })?;
                    calls.push(ToolCall {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments,
                    });
                }
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
            Some("content_filter") => StopReason::StopSequence,
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

impl std::fmt::Debug for OpenAiCompatProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatProvider")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("max_context", &self.max_context)
            .finish_non_exhaustive()
    }
}

// OpenAI API response types

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: OpenAiUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCall {
    id: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

// Streaming response types

#[derive(Debug, Deserialize)]
struct OpenAiStreamEvent {
    choices: Vec<OpenAiStreamChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

/// Check whether a URL points to a local endpoint (localhost, 127.0.0.1, etc.)
/// where an API key is typically not required.
fn is_local_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("localhost") || lower.contains("127.0.0.1") || lower.contains("[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCallResult;

    #[test]
    fn test_lm_studio_creation() {
        let provider = OpenAiCompatProvider::lm_studio();
        assert_eq!(provider.model(), "local-model");
        assert!(provider.api_key.is_none());
        assert!(provider.base_url.contains("localhost:1234"));
    }

    #[test]
    fn test_openai_creation() {
        let provider = OpenAiCompatProvider::openai("sk-test", "gpt-4");
        assert_eq!(provider.model(), "gpt-4");
        assert!(provider.api_key.is_some());
        assert!(provider.base_url.contains("api.openai.com"));
    }

    #[test]
    fn test_custom_provider() {
        let provider = OpenAiCompatProvider::custom(
            "http://my-server:8080/v1/chat/completions",
            Some("my-key"),
            "my-model",
        );
        assert_eq!(provider.model(), "my-model");
        assert_eq!(
            provider.base_url,
            "http://my-server:8080/v1/chat/completions"
        );
    }

    #[tokio::test]
    async fn test_invalid_api_key_characters() {
        let provider = OpenAiCompatProvider::openai("invalid\nkey", "gpt-4");
        let Err(err_complete) = provider.complete(&[], &[], "").await else {
            panic!("Expected error");
        };
        assert!(
            matches!(err_complete, LlmError::ApiRequestFailed(ref msg) if msg.contains("Invalid API key characters"))
        );

        let Err(err_stream) = provider.stream(&[], &[], "").await else {
            panic!("Expected error");
        };
        assert!(
            matches!(err_stream, LlmError::ApiRequestFailed(ref msg) if msg.contains("Invalid API key characters"))
        );
    }

    #[test]
    fn test_message_conversion() {
        let msg = Message::user("Hello");
        let converted = convert_message(&msg);
        assert_eq!(converted["role"], "user");
        assert_eq!(converted["content"], "Hello");
    }

    #[test]
    fn test_build_request() {
        let provider = OpenAiCompatProvider::lm_studio();
        let messages = vec![Message::user("Hi")];
        let request = provider.build_request(&messages, &[], "Be helpful", false);

        assert_eq!(request["model"], "local-model");
        assert_eq!(request["stream"], false);
        assert!(request["messages"].as_array().unwrap().len() >= 2); // system + user
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
}
