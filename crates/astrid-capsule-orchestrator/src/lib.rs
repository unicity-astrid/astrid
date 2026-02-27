#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![warn(missing_docs)]

//! Default orchestrator capsule for Astrid OS.
//!
//! Replaces the monolithic `astrid-runtime::execution` loop with an
//! event-driven state machine. Each IPC event invocation is stateless;
//! all conversation state is persisted to KV between invocations.
//!
//! # State Machine
//!
//! ```text
//! Idle → AwaitingIdentity → Streaming → AwaitingTools → Streaming → ... → Idle
//! ```
//!
//! The orchestrator contains no inference logic. It defines the control
//! flow that coordinates Identity, Provider, and Tool Router capsules
//! over the event bus.

use astrid_events::ipc::IpcPayload;
use astrid_events::llm::{
    LlmToolDefinition, Message, MessageContent, MessageRole, StreamEvent, ToolCall, ToolCallResult,
};
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// KV key prefix for the persisted session state.
///
/// State is keyed as `orchestrator.session.{session_id}` to support
/// concurrent sessions. Currently uses a fixed `"default"` session
/// until `UserInput` carries a session ID (Phase 8).
const STATE_KEY_PREFIX: &str = "orchestrator.session";

/// Default session ID used when the IPC payload does not specify one.
const DEFAULT_SESSION_ID: &str = "default";

/// Build the KV key for a session's persisted state.
fn state_key(session_id: &str) -> String {
    format!("{STATE_KEY_PREFIX}.{session_id}")
}

/// State machine phase for the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    /// No active turn. Waiting for user input.
    Idle,
    /// Waiting for the identity capsule to return the system prompt.
    AwaitingIdentity,
    /// Streaming tokens/tool calls from the LLM provider.
    Streaming,
    /// Waiting for all pending tool executions to complete.
    AwaitingTools,
}

/// A tool call being accumulated from stream deltas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolCall {
    /// Tool call ID from the LLM.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Accumulated JSON argument string (appended from deltas).
    pub args_json: String,
    /// Whether this tool call's stream has ended (ContentBlockStop received).
    pub complete: bool,
}

/// A tool call that has been dispatched and is awaiting a result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchedToolCall {
    /// Tool call ID.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Parsed arguments.
    pub arguments: serde_json::Value,
    /// Result, filled in when `tool.execute.result` arrives.
    pub result: Option<ToolCallResult>,
}

/// Persisted session state for the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Current state machine phase.
    pub phase: Phase,
    /// Conversation message history.
    pub messages: Vec<Message>,
    /// System prompt from the identity capsule (cached per turn).
    pub system_prompt: String,
    /// Request ID for the current LLM generation.
    pub request_id: Uuid,
    /// Accumulated response text from the current LLM stream.
    pub response_text: String,
    /// Tool calls being accumulated from stream deltas.
    pub pending_stream_tools: Vec<PendingToolCall>,
    /// Tool calls that have been dispatched for execution.
    pub dispatched_tools: Vec<DispatchedToolCall>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            messages: Vec::new(),
            system_prompt: String::new(),
            request_id: Uuid::nil(),
            response_text: String::new(),
            pending_stream_tools: Vec::new(),
            dispatched_tools: Vec::new(),
        }
    }
}

impl SessionState {
    /// Load state from KV for the given session, or create default if not present.
    fn load(session_id: &str) -> Self {
        let key = state_key(session_id);
        kv::get_json::<Self>(&key).unwrap_or_else(|e| {
            let _ = sys::log(
                "error",
                format!("Failed to load session state, resetting: {e}"),
            );
            Self::default()
        })
    }

    /// Persist state to KV for the given session.
    fn save(&self, session_id: &str) -> Result<(), SysError> {
        let key = state_key(session_id);
        kv::set_json(&key, self)
    }

    /// Reset per-turn accumulators for a new LLM generation round.
    fn reset_turn(&mut self) {
        self.response_text.clear();
        self.pending_stream_tools.clear();
        self.dispatched_tools.clear();
        self.request_id = Uuid::new_v4();
    }
}

/// Default orchestrator capsule.
#[derive(Default)]
pub struct Orchestrator;

#[capsule]
impl Orchestrator {
    /// Handles `user.prompt` events from frontends (CLI, Telegram, etc.).
    ///
    /// Adds the user message to conversation history, then requests the
    /// system prompt from the identity capsule.
    #[astrid::interceptor("handle_user_prompt")]
    pub fn handle_user_prompt(&self, payload: IpcPayload) -> Result<(), SysError> {
        let text = match payload {
            IpcPayload::UserInput { text, .. } => text,
            _ => return Ok(()),
        };

        if text.trim().is_empty() {
            return Ok(());
        }

        let mut state = SessionState::load(DEFAULT_SESSION_ID);

        // Add user message to history
        state.messages.push(Message {
            role: MessageRole::User,
            content: MessageContent::Text(text),
        });

        // Reset per-turn state
        state.reset_turn();
        state.phase = Phase::AwaitingIdentity;
        state.save(DEFAULT_SESSION_ID)?;

        // Request system prompt from the identity capsule.
        // The identity capsule reads workspace config and publishes
        // `identity.response.ready` with the assembled prompt.
        ipc::publish_json(
            "identity.request.build",
            &serde_json::json!({
                "workspace_root": sys::get_config_string("workspace_root").unwrap_or_default(),
            }),
        )?;

        Ok(())
    }

    /// Handles `identity.response.ready` events from the identity capsule.
    ///
    /// Receives the assembled system prompt and publishes an LLM generation
    /// request to the provider capsule.
    #[astrid::interceptor("handle_identity_response")]
    pub fn handle_identity_response(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let mut state = SessionState::load(DEFAULT_SESSION_ID);

        if state.phase != Phase::AwaitingIdentity {
            return Ok(());
        }

        // Extract the prompt from the identity capsule's BuildResponse
        let prompt = payload
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        state.system_prompt = prompt;
        state.phase = Phase::Streaming;
        state.save(DEFAULT_SESSION_ID)?;

        Self::publish_llm_request(&state)
    }

    /// Handles `llm.stream.anthropic` events from the LLM provider capsule.
    ///
    /// Accumulates text deltas and tool call deltas. When `StreamEvent::Done`
    /// arrives, evaluates whether to dispatch tool calls or emit the final
    /// response.
    #[astrid::interceptor("handle_llm_stream")]
    pub fn handle_llm_stream(&self, payload: IpcPayload) -> Result<(), SysError> {
        let (request_id, event) = match payload {
            IpcPayload::LlmStreamEvent { request_id, event } => (request_id, event),
            _ => return Ok(()),
        };

        let mut state = SessionState::load(DEFAULT_SESSION_ID);

        if state.phase != Phase::Streaming {
            return Ok(());
        }

        // Verify this stream belongs to our current request
        if state.request_id != request_id {
            return Ok(());
        }

        match event {
            StreamEvent::TextDelta(text) => {
                state.response_text.push_str(&text);
                // Forward to frontend for real-time display
                let _ = ipc::publish_json(
                    "agent.stream.delta",
                    &IpcPayload::AgentResponse {
                        text,
                        is_final: false,
                    },
                );
            }
            StreamEvent::ToolCallStart { id, name } => {
                state.pending_stream_tools.push(PendingToolCall {
                    id,
                    name,
                    args_json: String::new(),
                    complete: false,
                });
            }
            StreamEvent::ToolCallDelta { id, args_delta } => {
                if let Some(tc) = state.pending_stream_tools.iter_mut().find(|t| t.id == id) {
                    tc.args_json.push_str(&args_delta);
                }
            }
            StreamEvent::ToolCallEnd { id } => {
                if let Some(tc) = state.pending_stream_tools.iter_mut().find(|t| t.id == id) {
                    tc.complete = true;
                }
            }
            StreamEvent::Done => {
                state.save(DEFAULT_SESSION_ID)?;
                return Self::handle_stream_done(&mut state);
            }
            StreamEvent::Error(err) => {
                let _ = sys::log("error", format!("LLM stream error: {err}"));
                // Publish error to frontend and reset to idle
                let _ = ipc::publish_json(
                    "agent.response",
                    &IpcPayload::AgentResponse {
                        text: format!("LLM error: {err}"),
                        is_final: true,
                    },
                );
                state.phase = Phase::Idle;
                state.save(DEFAULT_SESSION_ID)?;
                return Ok(());
            }
            // Usage and ReasoningDelta are informational, no state change needed
            _ => {}
        }

        state.save(DEFAULT_SESSION_ID)?;
        Ok(())
    }

    /// Handles `tool.execute.result` events from the tool router.
    ///
    /// Records the result for the completed tool call. When all dispatched
    /// tool calls have results, appends them to conversation history and
    /// publishes the next LLM generation request.
    #[astrid::interceptor("handle_tool_result")]
    pub fn handle_tool_result(&self, payload: IpcPayload) -> Result<(), SysError> {
        let (call_id, result) = match payload {
            IpcPayload::ToolExecuteResult { call_id, result } => (call_id, result),
            _ => return Ok(()),
        };

        let mut state = SessionState::load(DEFAULT_SESSION_ID);

        if state.phase != Phase::AwaitingTools {
            return Ok(());
        }

        // Record the result for this tool call
        if let Some(tc) = state.dispatched_tools.iter_mut().find(|t| t.id == call_id) {
            tc.result = Some(result);
        }

        // Check if all dispatched tools have results
        let all_done = state.dispatched_tools.iter().all(|t| t.result.is_some());
        if !all_done {
            state.save(DEFAULT_SESSION_ID)?;
            return Ok(());
        }

        // All tool results received. Add assistant message with tool calls,
        // then add each tool result as a message, and start next LLM turn.
        let tool_calls: Vec<ToolCall> = state
            .dispatched_tools
            .iter()
            .map(|t| ToolCall {
                id: t.id.clone(),
                name: t.name.clone(),
                arguments: t.arguments.clone(),
            })
            .collect();
        state
            .messages
            .push(Message::assistant_with_tools(tool_calls));

        for tc in &state.dispatched_tools {
            if let Some(ref result) = tc.result {
                state.messages.push(Message {
                    role: MessageRole::Tool,
                    content: MessageContent::ToolResult(result.clone()),
                });
            }
        }

        // Reset per-turn accumulators and start next LLM generation
        state.reset_turn();
        state.phase = Phase::Streaming;
        state.save(DEFAULT_SESSION_ID)?;

        Self::publish_llm_request(&state)
    }
}

impl Orchestrator {
    /// Called when the LLM stream finishes. Evaluates whether to dispatch
    /// tool calls or emit the final response.
    fn handle_stream_done(state: &mut SessionState) -> Result<(), SysError> {
        let has_tool_calls = !state.pending_stream_tools.is_empty();

        if has_tool_calls {
            // Parse accumulated tool calls and dispatch to the tool router
            let mut dispatched = Vec::new();

            for tc in &state.pending_stream_tools {
                let arguments: serde_json::Value = match serde_json::from_str(&tc.args_json) {
                    Ok(args) => args,
                    Err(e) => {
                        let _ = sys::log(
                            "warn",
                            format!(
                                "Failed to parse tool arguments for {}: {e}. Defaulting to empty object.",
                                tc.name
                            ),
                        );
                        serde_json::Value::Object(serde_json::Map::new())
                    }
                };

                dispatched.push(DispatchedToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: arguments.clone(),
                    result: None,
                });

                // Publish tool execution request to the router
                ipc::publish_json(
                    "tool.request.execute",
                    &IpcPayload::ToolExecuteRequest {
                        call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        arguments,
                    },
                )?;
            }

            state.dispatched_tools = dispatched;
            state.pending_stream_tools.clear();
            state.phase = Phase::AwaitingTools;
            state.save(DEFAULT_SESSION_ID)?;
        } else if !state.response_text.is_empty() {
            // Text response with no tool calls — conversation turn complete
            state
                .messages
                .push(Message::assistant(&state.response_text));

            // Publish final response to frontends
            ipc::publish_json(
                "agent.response",
                &IpcPayload::AgentResponse {
                    text: state.response_text.clone(),
                    is_final: true,
                },
            )?;

            state.phase = Phase::Idle;
            state.save(DEFAULT_SESSION_ID)?;
        } else {
            // Empty response — done
            state.phase = Phase::Idle;
            state.save(DEFAULT_SESSION_ID)?;
        }

        Ok(())
    }

    /// Publish an LLM generation request to the provider capsule.
    fn publish_llm_request(state: &SessionState) -> Result<(), SysError> {
        let model =
            sys::get_config_string("model").unwrap_or_else(|_| "claude-sonnet-4-20250514".into());

        let tools = Self::load_tool_schemas();

        ipc::publish_json(
            "llm.request.generate.anthropic",
            &IpcPayload::LlmRequest {
                request_id: state.request_id,
                model,
                messages: state.messages.clone(),
                tools,
                system: state.system_prompt.clone(),
            },
        )
    }

    /// Load tool schemas from KV.
    ///
    /// The kernel writes available tool schemas to the `tool_schemas` KV key
    /// when capsules are loaded. Returns an empty vec if no schemas are available.
    fn load_tool_schemas() -> Vec<LlmToolDefinition> {
        kv::get_json::<Vec<LlmToolDefinition>>("tool_schemas").unwrap_or_default()
    }
}
