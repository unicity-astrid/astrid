#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![warn(missing_docs)]

//! Anthropic LLM provider capsule.
//!
//! Subscribes to `llm.request.generate.anthropic` IPC events, calls the
//! Anthropic Messages API via the HTTP airlock, parses the SSE streaming
//! response, and publishes standardized `llm.stream.anthropic` events back
//! to the event bus.

mod schemas;

use astrid_events::ipc::IpcPayload;
use astrid_events::llm::{Message, MessageContent, MessageRole, StreamEvent};
use astrid_sdk::prelude::*;
use schemas::{ContentBlock, Delta, HttpRequest, HttpResponse, StreamingEvent};
use serde_json::Value;
use uuid::Uuid;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic LLM provider capsule.
#[derive(Default)]
pub struct AnthropicProvider;

#[capsule]
impl AnthropicProvider {
    /// Handles incoming LLM generation requests destined for the Anthropic API.
    #[astrid::interceptor("handle_llm_request")]
    pub fn handle_llm_request(&self, req: IpcPayload) -> Result<(), SysError> {
        if let IpcPayload::LlmRequest {
            request_id,
            messages,
            tools,
            system,
            ..
        } = req
        {
            if let Err(e) = Self::execute_request(request_id, &messages, &tools, &system) {
                let _ = sys::log("error", format!("LLM request failed: {e}"));
                let _ = ipc::publish_json(
                    "llm.stream.anthropic",
                    &IpcPayload::LlmStreamEvent {
                        request_id,
                        event: StreamEvent::Error(e.to_string()),
                    },
                );
            }
        }
        Ok(())
    }
}

impl AnthropicProvider {
    /// Build and send the HTTP request to the Anthropic Messages API,
    /// then parse the SSE response and publish stream events.
    fn execute_request(
        request_id: Uuid,
        messages: &[Message],
        tools: &[astrid_events::llm::LlmToolDefinition],
        system: &str,
    ) -> Result<(), SysError> {
        let api_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(Self::convert_message)
            .collect();

        let mut request_body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 8192,
            "messages": api_messages,
            "stream": true,
        });

        if !system.is_empty() {
            request_body["system"] = Value::String(system.to_string());
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
            request_body["tools"] = Value::Array(api_tools);
        }

        let api_key = sys::get_config_string("anthropic_api_key")
            .or_else(|_| sys::get_config_string("api_key"))
            .unwrap_or_default();

        if api_key.is_empty() {
            return Err(SysError::ApiError(
                "anthropic_api_key not configured".into(),
            ));
        }

        let mut headers = std::collections::HashMap::new();
        headers.insert("x-api-key".to_string(), api_key);
        headers.insert(
            "anthropic-version".to_string(),
            ANTHROPIC_VERSION.to_string(),
        );
        headers.insert("content-type".to_string(), "application/json".to_string());

        let req = HttpRequest {
            url: ANTHROPIC_API_URL.to_string(),
            method: "POST".to_string(),
            headers,
            body: Some(serde_json::to_string(&request_body)?),
        };

        let req_bytes = serde_json::to_vec(&req)?;
        let res_bytes = http::request_bytes(&req_bytes)?;

        let res: HttpResponse = serde_json::from_slice(&res_bytes)
            .map_err(|e| SysError::ApiError(format!("failed to parse HTTP response: {e}")))?;

        if res.status != 200 {
            return Err(SysError::ApiError(format!(
                "Anthropic API error ({}): {}",
                res.status, res.body
            )));
        }

        Self::parse_sse_stream(request_id, &res.body)
    }

    /// Parse the SSE response body and publish standardized stream events.
    fn parse_sse_stream(request_id: Uuid, body: &str) -> Result<(), SysError> {
        let mut buffer = body;
        let mut current_tool_id = String::new();

        while let Some(event_end) = buffer.find("\n\n") {
            let event_data = &buffer[..event_end];
            buffer = &buffer[(event_end + 2)..];

            for line in event_data.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        Self::publish_stream(request_id, StreamEvent::Done)?;
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<StreamingEvent>(data) {
                        Self::handle_sse_event(request_id, event, &mut current_tool_id)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Convert a single Anthropic SSE event into standardized stream events.
    fn handle_sse_event(
        request_id: Uuid,
        event: StreamingEvent,
        current_tool_id: &mut String,
    ) -> Result<(), SysError> {
        match event {
            StreamingEvent::ContentBlockStart { content_block, .. } => match content_block {
                ContentBlock::Text { .. } => {}
                ContentBlock::ToolUse { id, name, .. } => {
                    *current_tool_id = id.clone();
                    Self::publish_stream(request_id, StreamEvent::ToolCallStart { id, name })?;
                }
            },
            StreamingEvent::ContentBlockDelta { delta, .. } => match delta {
                Delta::TextDelta { text } => {
                    Self::publish_stream(request_id, StreamEvent::TextDelta(text))?;
                }
                Delta::InputJsonDelta { partial_json } => {
                    Self::publish_stream(
                        request_id,
                        StreamEvent::ToolCallDelta {
                            id: current_tool_id.clone(),
                            args_delta: partial_json,
                        },
                    )?;
                }
            },
            StreamingEvent::ContentBlockStop { .. } => {
                if !current_tool_id.is_empty() {
                    Self::publish_stream(
                        request_id,
                        StreamEvent::ToolCallEnd {
                            id: current_tool_id.clone(),
                        },
                    )?;
                    current_tool_id.clear();
                }
            }
            StreamingEvent::MessageDelta {
                usage: Some(usage), ..
            } => {
                Self::publish_stream(
                    request_id,
                    StreamEvent::Usage {
                        input_tokens: usage.input_tokens.unwrap_or(0),
                        output_tokens: usage.output_tokens,
                    },
                )?;
            }
            StreamingEvent::MessageStop => {
                Self::publish_stream(request_id, StreamEvent::Done)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Publish a stream event to the event bus with the original request ID.
    fn publish_stream(request_id: Uuid, event: StreamEvent) -> Result<(), SysError> {
        ipc::publish_json(
            "llm.stream.anthropic",
            &IpcPayload::LlmStreamEvent { request_id, event },
        )
    }

    /// Convert an `astrid_events::llm::Message` to the Anthropic API JSON format.
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
            }
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
            }
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
            }
            MessageContent::MultiPart(parts) => {
                let content: Vec<Value> = parts
                    .iter()
                    .map(|p| match p {
                        astrid_events::llm::ContentPart::Text { text } => {
                            serde_json::json!({"type": "text", "text": text})
                        }
                        astrid_events::llm::ContentPart::Image { media_type, data } => {
                            serde_json::json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": data,
                                }
                            })
                        }
                    })
                    .collect();

                serde_json::json!({
                    "role": "user",
                    "content": content,
                })
            }
        }
    }
}
