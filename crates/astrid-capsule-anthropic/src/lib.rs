#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

mod schemas;

use astrid_events::ipc::IpcPayload;
use astrid_events::llm::{Message, MessageContent, MessageRole, StreamEvent};
use astrid_sdk::prelude::*;
use schemas::{ContentBlock, Delta, StreamingEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Serialize)]
struct HttpRequest {
    url: String,
    method: String,
    headers: std::collections::HashMap<String, String>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct HttpResponse {
    status: u16,
    #[allow(dead_code)]
    headers: std::collections::HashMap<String, String>,
    body: String,
}

#[derive(Default)]
pub struct AnthropicCapsule;

#[capsule]
impl AnthropicCapsule {
    #[astrid::interceptor("handle_llm_request")]
    pub fn handle_llm_request(&self, req: IpcPayload) -> Result<(), SysError> {
        if let IpcPayload::LlmRequest {
            messages,
            tools,
            system,
            ..
        } = req
        {
            if let Err(e) = Self::execute_request(messages, tools, &system) {
                sys::log("error", format!("Failed to handle LLM request: {e}"))?;
                let _ = ipc::publish_json(
                    "llm.stream.anthropic",
                    &IpcPayload::LlmStreamEvent {
                        request_id: uuid::Uuid::new_v4(), // We should pass request_id from the original request
                        event: StreamEvent::Error(e.to_string()),
                    },
                );
            }
        }
        Ok(())
    }
}

impl AnthropicCapsule {
    fn execute_request(
        messages: Vec<Message>,
        tools: Vec<astrid_events::llm::LlmToolDefinition>,
        system: &str,
    ) -> Result<(), SysError> {
        // Build API request
        let api_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(Self::convert_message)
            .collect();

        let mut request_body = serde_json::json!({
            "model": "claude-3-5-sonnet-20241022",
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
            .unwrap_or_else(|_| sys::get_config_string("api_key").unwrap_or_default());

        if api_key.is_empty() {
            return Err(SysError::ApiError("API key not configured".into()));
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
            .map_err(|e| SysError::ApiError(format!("Failed to parse HTTP response: {e}")))?;

        if res.status != 200 {
            return Err(SysError::ApiError(format!(
                "Anthropic API Error ({}): {}",
                res.status, res.body
            )));
        }

        // Parse SSE Stream
        let mut buffer = res.body;
        let mut current_tool_id = String::new();

        while let Some(event_end) = buffer.find("\n\n") {
            let event_data = buffer[..event_end].to_string();
            buffer = buffer[(event_end + 2)..].to_string();

            for line in event_data.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        Self::publish_event(StreamEvent::Done)?;
                        continue;
                    }

                    if let Ok(event) = serde_json::from_str::<StreamingEvent>(data) {
                        match event {
                            StreamingEvent::ContentBlockStart { content_block, .. } => {
                                match content_block {
                                    ContentBlock::Text { .. } => {},
                                    ContentBlock::ToolUse { id, name, .. } => {
                                        current_tool_id = id.clone();
                                        Self::publish_event(StreamEvent::ToolCallStart {
                                            id,
                                            name,
                                        })?;
                                    },
                                }
                            },
                            StreamingEvent::ContentBlockDelta { delta, .. } => match delta {
                                Delta::TextDelta { text } => {
                                    Self::publish_event(StreamEvent::TextDelta(text))?;
                                },
                                Delta::InputJsonDelta { partial_json } => {
                                    Self::publish_event(StreamEvent::ToolCallDelta {
                                        id: current_tool_id.clone(),
                                        args_delta: partial_json,
                                    })?;
                                },
                            },
                            StreamingEvent::ContentBlockStop { .. } => {
                                if !current_tool_id.is_empty() {
                                    Self::publish_event(StreamEvent::ToolCallEnd {
                                        id: current_tool_id.clone(),
                                    })?;
                                    current_tool_id.clear();
                                }
                            },
                            StreamingEvent::MessageDelta {
                                usage: Some(usage), ..
                            } => {
                                Self::publish_event(StreamEvent::Usage {
                                    input_tokens: usage.input_tokens.unwrap_or(0),
                                    output_tokens: usage.output_tokens,
                                })?;
                            },
                            StreamingEvent::MessageStop => {
                                Self::publish_event(StreamEvent::Done)?;
                            },
                            _ => {},
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn publish_event(event: StreamEvent) -> Result<(), SysError> {
        let payload = IpcPayload::LlmStreamEvent { 
            request_id: uuid::Uuid::new_v4(), // Fix later to pass ID
            event 
        };
        ipc::publish_json("llm.stream.anthropic", &payload)
    }

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
                        let id = &c.id;
                        let name = &c.name;
                        let input = &c.arguments;
                        serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        })
                    })
                    .collect();

                serde_json::json!({
                    "role": "assistant",
                    "content": content,
                })
            },
            MessageContent::ToolResult(result) => {
                let call_id = &result.call_id;
                let content = &result.content;
                let is_error = result.is_error;

                serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content,
                        "is_error": is_error,
                    }],
                })
            },
            MessageContent::MultiPart(parts) => {
                let content: Vec<Value> = parts
                    .iter()
                    .map(|p| {
                        match p {
                            astrid_events::llm::ContentPart::Text { text } => {
                                serde_json::json!({"type": "text", "text": text})
                            },
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
                        }
                    })
                    .collect();

                serde_json::json!({
                    "role": "user",
                    "content": content,
                })
            },
        }
    }
}