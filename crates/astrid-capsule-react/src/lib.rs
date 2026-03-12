#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![warn(missing_docs)]

//! ReAct loop capsule for Astrid OS.
//!
//! Stateless coordinator that drives the reasoning-and-action loop:
//! fetch history from session, run it through identity + prompt builder,
//! send to LLM, collect response, dispatch tools, loop. Sends clean
//! results back to the session capsule at turn boundaries.
//!
//! # State Machine
//!
//! ```text
//! Idle -> AwaitingIdentity -> AwaitingPromptBuild -> Streaming -> AwaitingTools -> Streaming -> ... -> Idle
//! ```
//!
//! The react loop contains no inference logic. It defines the control
//! flow that coordinates Session, Identity, Prompt Builder, Provider,
//! and Tool Router capsules over the event bus.

use astrid_events::ipc::IpcPayload;
use astrid_events::llm::{
    LlmToolDefinition, Message, MessageContent, MessageRole, StreamEvent, ToolCall, ToolCallResult,
};
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// KV key prefix for the persisted turn state.
///
/// Keyed as `react.turn.{session_id}`. This is ephemeral per-turn
/// control flow state, not conversation history (that lives in the
/// session capsule).
const TURN_KEY_PREFIX: &str = "react.turn";

/// Default session ID used when the IPC payload does not specify one.
const DEFAULT_SESSION_ID: &str = "default";

/// KV key prefix for request_id -> session_id correlation.
const REQUEST_SESSION_PREFIX: &str = "react.req2sess";

/// KV key prefix for call_id -> session_id correlation.
const CALL_SESSION_PREFIX: &str = "react.call2sess";

/// Default timeout in milliseconds for session capsule requests.
const DEFAULT_SESSION_TIMEOUT_MS: u64 = 2_000;

/// Build the KV key for a session's turn state.
fn turn_key(session_id: &str) -> String {
    format!("{TURN_KEY_PREFIX}.{session_id}")
}

/// Store a request_id -> session_id mapping so LLM stream handlers
/// can resolve the owning session from the stream's request_id.
fn store_request_session(request_id: &Uuid, session_id: &str) -> Result<(), SysError> {
    let key = format!("{REQUEST_SESSION_PREFIX}.{request_id}");
    kv::set_bytes(&key, session_id.as_bytes())
}

/// Look up session_id from a request_id.
fn lookup_session_by_request(request_id: &Uuid) -> Option<String> {
    let key = format!("{REQUEST_SESSION_PREFIX}.{request_id}");
    kv::get_bytes(&key)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
}

/// Store call_id -> session_id mappings so tool result handlers
/// can resolve the owning session from the tool's call_id.
fn store_call_sessions(call_ids: &[String], session_id: &str) -> Result<(), SysError> {
    for call_id in call_ids {
        let key = format!("{CALL_SESSION_PREFIX}.{call_id}");
        kv::set_bytes(&key, session_id.as_bytes())?;
    }
    Ok(())
}

/// Look up session_id from a tool call_id.
fn lookup_session_by_call(call_id: &str) -> Option<String> {
    let key = format!("{CALL_SESSION_PREFIX}.{call_id}");
    kv::get_bytes(&key)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
}

/// Clean up a request_id -> session_id mapping after the LLM stream completes.
fn delete_request_session(request_id: &Uuid) {
    let key = format!("{REQUEST_SESSION_PREFIX}.{request_id}");
    if let Err(e) = kv::delete(&key) {
        let _ = sys::log("warn", format!("Failed to delete req2sess key '{key}': {e}"));
    }
}

/// Clean up call_id -> session_id mappings after all tool results are collected.
fn delete_call_sessions(call_ids: &[String]) {
    for call_id in call_ids {
        let key = format!("{CALL_SESSION_PREFIX}.{call_id}");
        if let Err(e) = kv::delete(&key) {
            let _ = sys::log("warn", format!("Failed to delete call2sess key '{key}': {e}"));
        }
    }
}

/// Resolve the session timeout from capsule config, falling back to default.
fn session_timeout_ms() -> u64 {
    match sys::get_config_string("session_timeout_ms") {
        Ok(s) => match s.parse() {
            Ok(v) => v,
            Err(e) => {
                let _ = sys::log(
                    "warn",
                    format!("Invalid session_timeout_ms config '{s}': {e}, using default {DEFAULT_SESSION_TIMEOUT_MS}ms"),
                );
                DEFAULT_SESSION_TIMEOUT_MS
            },
        },
        Err(_) => DEFAULT_SESSION_TIMEOUT_MS,
    }
}

/// State machine phase for the react loop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum Phase {
    /// No active turn. Waiting for user input.
    Idle,
    /// Waiting for the identity capsule to return the system prompt.
    AwaitingIdentity,
    /// Waiting for the prompt builder capsule to assemble the final prompt.
    AwaitingPromptBuild,
    /// Streaming tokens/tool calls from the LLM provider.
    Streaming,
    /// Waiting for all pending tool executions to complete.
    AwaitingTools,
}

/// A tool call being accumulated from stream deltas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingToolCall {
    /// Tool call ID from the LLM.
    id: String,
    /// Tool name.
    name: String,
    /// Accumulated JSON argument string (appended from deltas).
    args_json: String,
    /// Whether this tool call's stream has ended (ContentBlockStop received).
    complete: bool,
}

/// A tool call that has been dispatched and is awaiting a result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DispatchedToolCall {
    /// Tool call ID.
    id: String,
    /// Tool name.
    name: String,
    /// Parsed arguments.
    arguments: serde_json::Value,
    /// Result, filled in when `tool.execute.result` arrives.
    result: Option<ToolCallResult>,
}

/// Ephemeral per-turn state for the react loop.
///
/// This is control flow state, not conversation history. History
/// lives in the session capsule and is fetched on demand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TurnState {
    /// Session ID for this conversation.
    session_id: String,
    /// Current state machine phase.
    phase: Phase,
    /// System prompt from the identity capsule (ephemeral, for this turn only).
    system_prompt: String,
    /// Request ID for the current LLM generation.
    request_id: Uuid,
    /// Accumulated response text from the current LLM stream.
    response_text: String,
    /// Tool calls being accumulated from stream deltas.
    pending_stream_tools: Vec<PendingToolCall>,
    /// Tool calls that have been dispatched for execution.
    dispatched_tools: Vec<DispatchedToolCall>,
}

impl Default for TurnState {
    fn default() -> Self {
        Self {
            session_id: DEFAULT_SESSION_ID.into(),
            phase: Phase::Idle,
            system_prompt: String::new(),
            request_id: Uuid::nil(),
            response_text: String::new(),
            pending_stream_tools: Vec::new(),
            dispatched_tools: Vec::new(),
        }
    }
}

impl TurnState {
    /// Load turn state from KV, or create default if not present.
    fn load(session_id: &str) -> Self {
        let key = turn_key(session_id);
        let mut state = kv::get_json::<Self>(&key).unwrap_or_else(|e| {
            let _ = sys::log(
                "error",
                format!("Failed to load turn state, resetting: {e}"),
            );
            Self::default()
        });
        // Ensure session_id matches what was requested (handles default case)
        state.session_id = session_id.into();
        state
    }

    /// Persist turn state to KV, keyed by the actual session ID.
    fn save(&self) -> Result<(), SysError> {
        let key = turn_key(&self.session_id);
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

/// ReAct loop capsule.
#[derive(Default)]
pub struct ReactLoop;

#[capsule]
impl ReactLoop {
    /// Handles `user.prompt` events from platforms (CLI, Telegram, etc.).
    ///
    /// Appends the user message to the session capsule, fetches history,
    /// then requests the system prompt from the identity capsule.
    #[astrid::interceptor("handle_user_prompt")]
    pub fn handle_user_prompt(&self, payload: IpcPayload) -> Result<(), SysError> {
        let (text, session_id, context) = match payload {
            IpcPayload::UserInput {
                text,
                session_id,
                context,
            } => (text, session_id, context),
            _ => return Ok(()),
        };

        // Check for cancel signal before the empty-text guard, since
        // cancel is sent as empty text with context.action = "cancel_turn".
        if let Some(ref ctx) = context {
            if ctx.get("action").and_then(|v| v.as_str()) == Some("cancel_turn") {
                return Self::handle_cancel(&session_id);
            }
        }

        if text.trim().is_empty() {
            return Ok(());
        }

        // Warn when using the default session ID - may indicate an
        // unpatched frontend that doesn't send session_id yet.
        if session_id == DEFAULT_SESSION_ID {
            let _ = sys::log(
                "warn",
                "UserInput using default session_id - frontend may not be sending session_id",
            );
        }

        // Load or create TurnState keyed by the actual session ID.
        let mut state = TurnState::load(&session_id);

        // Append the user message to session atomically. The returned
        // history is not cached - downstream handlers fetch fresh.
        Self::fetch_messages_with_append(
            &state.session_id,
            &[Message {
                role: MessageRole::User,
                content: MessageContent::Text(text),
            }],
        )?;

        // Clean up any in-flight mappings from a previous interrupted turn
        // before resetting, otherwise stale req2sess/call2sess entries leak.
        if state.phase != Phase::Idle {
            Self::cleanup_inflight_mappings(&state);
        }
        state.reset_turn();
        state.phase = Phase::AwaitingIdentity;
        state.save()?;

        // Request system prompt from the identity capsule.
        // session_id is threaded through so the response echoes it back.
        ipc::publish_json(
            "identity.request.build",
            &serde_json::json!({
                "workspace_root": sys::get_config_string("workspace_root").unwrap_or_default(),
                "session_id": state.session_id,
            }),
        )?;

        Ok(())
    }

    /// Handles `identity.response.ready` events from the identity capsule.
    ///
    /// Receives the assembled system prompt and sends it to the prompt
    /// builder capsule for capsule hook interception before LLM generation.
    #[astrid::interceptor("handle_identity_response")]
    pub fn handle_identity_response(&self, payload: serde_json::Value) -> Result<(), SysError> {
        // Resolve session from the echoed session_id in the identity response.
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_SESSION_ID);

        let mut state = TurnState::load(session_id);

        if state.phase != Phase::AwaitingIdentity {
            return Ok(());
        }

        // Extract the prompt from the identity capsule's BuildResponse
        let prompt = payload
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        state.system_prompt = prompt.clone();
        state.phase = Phase::AwaitingPromptBuild;

        // Fetch messages from session to send to prompt builder for plugin
        // hook interception. The prompt builder's response does not echo
        // messages back, so handle_prompt_response fetches again. This
        // costs an extra session round-trip but keeps the prompt builder
        // response lean.
        let messages = Self::fetch_messages(&state.session_id)?;

        state.save()?;

        let model =
            sys::get_config_string("model").unwrap_or_else(|_| "claude-sonnet-4-20250514".into());

        // Derive the active provider from the registry's LLM topic.
        let llm_topic = Self::active_llm_topic();
        let provider = llm_topic
            .strip_prefix("llm.request.generate.")
            .unwrap_or("unknown")
            .to_string();

        // Send to prompt builder for plugin hook interception.
        // session_id is threaded through so the response echoes it back.
        ipc::publish_json(
            "prompt_builder.assemble",
            &serde_json::json!({
                "messages": messages,
                "system_prompt": prompt,
                "request_id": state.request_id.to_string(),
                "session_id": state.session_id,
                "model": model,
                "provider": provider,
            }),
        )
    }

    /// Handles `prompt_builder.response.assemble` events from the prompt builder.
    ///
    /// Receives the final assembled prompt (after capsule hooks) and publishes
    /// an LLM generation request to the provider capsule.
    #[astrid::interceptor("handle_prompt_response")]
    pub fn handle_prompt_response(&self, payload: serde_json::Value) -> Result<(), SysError> {
        // Resolve session from the echoed session_id in the prompt builder response.
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_SESSION_ID);

        let mut state = TurnState::load(session_id);

        if state.phase != Phase::AwaitingPromptBuild {
            return Ok(());
        }

        // Apply the assembled system prompt from the prompt builder.
        if let Some(prompt) = payload.get("system_prompt").and_then(|v| v.as_str()) {
            state.system_prompt = prompt.to_string();
        }

        // Fetch messages from session for prompt assembly.
        let mut messages = Self::fetch_messages(&state.session_id)?;

        // Apply user context prefix to the LOCAL COPY ONLY.
        // Session's copy stays clean - this is an ephemeral transform.
        if let Some(prefix) = payload.get("user_context_prefix").and_then(|v| v.as_str())
            && !prefix.is_empty()
            && let Some(last_user_msg) = messages
                .iter_mut()
                .rev()
                .find(|m| matches!(m.role, MessageRole::User))
            && let MessageContent::Text(ref mut text) = last_user_msg.content
        {
            *text = format!("{prefix}\n{text}");
        }

        state.phase = Phase::Streaming;
        state.save()?;

        Self::publish_llm_request(&state, &messages)
    }

    /// Handles `llm.stream.*` events from the LLM provider capsule.
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

        // Resolve session from the request_id -> session_id mapping
        // stored when the LLM request was published.
        let session_id = match lookup_session_by_request(&request_id) {
            Some(sid) => sid,
            None => return Ok(()), // Unknown request, ignore
        };

        let mut state = TurnState::load(&session_id);

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
                // Forward to platform for real-time display
                let _ = ipc::publish_json(
                    "agent.stream.delta",
                    &IpcPayload::AgentResponse {
                        text,
                        is_final: false,
                        session_id: state.session_id.clone(),
                    },
                );
            },
            StreamEvent::ToolCallStart { id, name } => {
                state.pending_stream_tools.push(PendingToolCall {
                    id,
                    name,
                    args_json: String::new(),
                    complete: false,
                });
            },
            StreamEvent::ToolCallDelta { id, args_delta } => {
                if let Some(tc) = state.pending_stream_tools.iter_mut().find(|t| t.id == id) {
                    tc.args_json.push_str(&args_delta);
                }
            },
            StreamEvent::ToolCallEnd { id } => {
                if let Some(tc) = state.pending_stream_tools.iter_mut().find(|t| t.id == id) {
                    tc.complete = true;
                }
            },
            StreamEvent::Done => {
                return Self::handle_stream_done(&mut state);
            },
            StreamEvent::Error(err) => {
                delete_request_session(&state.request_id);
                let _ = sys::log("error", format!("LLM stream error: {err}"));
                let _ = ipc::publish_json(
                    "agent.response",
                    &IpcPayload::AgentResponse {
                        text: format!("LLM error: {err}"),
                        is_final: true,
                        session_id: state.session_id.clone(),
                    },
                );
                state.phase = Phase::Idle;
                state.save()?;
                return Ok(());
            },
            // Usage and ReasoningDelta are informational, no state change needed
            _ => {},
        }

        state.save()?;
        Ok(())
    }

    /// Handles `tool.execute.result` events from the tool router.
    ///
    /// Records the result for the completed tool call. When all dispatched
    /// tool calls have results, appends them to session and publishes the
    /// next LLM generation request.
    #[astrid::interceptor("handle_tool_result")]
    pub fn handle_tool_result(&self, payload: IpcPayload) -> Result<(), SysError> {
        let (call_id, result) = match payload {
            IpcPayload::ToolExecuteResult { call_id, result } => (call_id, result),
            _ => return Ok(()),
        };

        // Resolve session from the call_id -> session_id mapping
        // stored when the tool call was dispatched.
        let session_id = match lookup_session_by_call(&call_id) {
            Some(sid) => sid,
            None => return Ok(()), // Unknown call, ignore
        };

        let mut state = TurnState::load(&session_id);

        if state.phase != Phase::AwaitingTools {
            return Ok(());
        }

        // Record the result for this tool call
        if let Some(tc) = state.dispatched_tools.iter_mut().find(|t| t.id == call_id) {
            tc.result = Some(result);
        }

        // Check if all dispatched tools have results.
        // Guard against vacuous truth: empty dispatched_tools means the
        // turn was reset (e.g. by a new user prompt) and this is a stale
        // tool result arriving late.
        let all_done = !state.dispatched_tools.is_empty()
            && state.dispatched_tools.iter().all(|t| t.result.is_some());
        if !all_done {
            state.save()?;
            return Ok(());
        }

        // Build clean messages for session.
        let tool_calls: Vec<ToolCall> = state
            .dispatched_tools
            .iter()
            .map(|t| ToolCall {
                id: t.id.clone(),
                name: t.name.clone(),
                arguments: t.arguments.clone(),
            })
            .collect();

        let mut session_messages = vec![Message::assistant_with_tools(tool_calls)];
        for tc in &state.dispatched_tools {
            if let Some(ref result) = tc.result {
                session_messages.push(Message {
                    role: MessageRole::Tool,
                    content: MessageContent::ToolResult(result.clone()),
                });
            }
        }

        // Fetch fresh history with atomic append-before-read BEFORE
        // deleting call2sess mappings. If this fails, mappings survive
        // and a retry from the same tool result can re-enter this path.
        let messages = Self::fetch_messages_with_append(&state.session_id, &session_messages)?;

        // Commit new state before removing mappings so a save failure
        // doesn't orphan the session with deleted mappings.
        let call_ids: Vec<String> = state.dispatched_tools.iter().map(|t| t.id.clone()).collect();
        state.reset_turn();
        state.phase = Phase::Streaming;
        state.save()?;

        // Safe to clean up now - new state is committed.
        delete_call_sessions(&call_ids);

        Self::publish_llm_request(&state, &messages)
    }

    /// Handle active model change from the registry capsule.
    ///
    /// Stores the new provider topic in KV so subsequent LLM requests
    /// route to the correct provider. Validates that the topic follows
    /// the expected `llm.request.generate.*` pattern as defense-in-depth.
    #[astrid::interceptor("handle_model_changed")]
    pub fn handle_model_changed(&self, payload: IpcPayload) -> Result<(), SysError> {
        if let IpcPayload::Custom { data } = payload {
            if let Some(topic) = data.get("request_topic").and_then(|t| t.as_str()) {
                if !topic.starts_with("llm.request.generate.") {
                    let _ = sys::log(
                        "warn",
                        format!("Rejected model change with invalid topic: {topic}"),
                    );
                    return Ok(());
                }
                kv::set_bytes("llm_provider_topic", topic.as_bytes())?;
            }
        } else {
            let _ = sys::log(
                "warn",
                "handle_model_changed: unexpected payload type, ignoring",
            );
        }
        Ok(())
    }
}

impl ReactLoop {
    /// Called when the LLM stream finishes. Evaluates whether to dispatch
    /// tool calls or emit the final response.
    fn handle_stream_done(state: &mut TurnState) -> Result<(), SysError> {
        // Clean up the request_id -> session_id mapping now that the stream is done.
        delete_request_session(&state.request_id);

        let has_tool_calls = !state.pending_stream_tools.is_empty();

        if has_tool_calls {
            // Two-phase tool dispatch: parse all tool calls first, then publish.
            // This prevents partial dispatch if a publish fails mid-loop.
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
                    },
                };

                dispatched.push(DispatchedToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments,
                    result: None,
                });
            }

            // Store call_id -> session_id mappings BEFORE publishing so
            // results that arrive immediately are never orphaned.
            let call_ids: Vec<String> = dispatched.iter().map(|t| t.id.clone()).collect();
            store_call_sessions(&call_ids, &state.session_id)?;

            state.dispatched_tools = dispatched;
            state.pending_stream_tools.clear();
            state.phase = Phase::AwaitingTools;
            if let Err(e) = state.save() {
                delete_call_sessions(&call_ids);
                return Err(e);
            }

            // Phase 2: publish all tool requests. On failure, clean up
            // the mappings we wrote so they don't leak.
            for tc in &state.dispatched_tools {
                if let Err(e) = ipc::publish_json(
                    "tool.request.execute",
                    &IpcPayload::ToolExecuteRequest {
                        call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                    },
                ) {
                    let _ = sys::log(
                        "error",
                        format!("Failed to dispatch tool {}: {e}", tc.name),
                    );
                    delete_call_sessions(&call_ids);
                    let _ = ipc::publish_json(
                        "agent.response",
                        &IpcPayload::AgentResponse {
                            text: format!("Failed to dispatch tool {}: {e}", tc.name),
                            is_final: true,
                            session_id: state.session_id.clone(),
                        },
                    );
                    state.phase = Phase::Idle;
                    state.save()?;
                    return Err(e);
                }
            }
        } else if !state.response_text.is_empty() {
            // Text response with no tool calls - conversation turn complete.
            // Use atomic append to confirm delivery to session.
            Self::fetch_messages_with_append(
                &state.session_id,
                &[Message::assistant(&state.response_text)],
            )?;

            // Publish final response to platforms
            ipc::publish_json(
                "agent.response",
                &IpcPayload::AgentResponse {
                    text: state.response_text.clone(),
                    is_final: true,
                    session_id: state.session_id.clone(),
                },
            )?;

            state.phase = Phase::Idle;
            state.save()?;
        } else {
            // Empty response
            state.phase = Phase::Idle;
            state.save()?;
        }

        Ok(())
    }

    /// Clean up in-flight KV correlation mappings for the current turn.
    ///
    /// Must be called before `reset_turn()` when interrupting an active turn,
    /// otherwise stale `req2sess` and `call2sess` entries accumulate.
    fn cleanup_inflight_mappings(state: &TurnState) {
        // Always clean request mapping if one exists (Streaming, AwaitingPromptBuild, etc.)
        if !state.request_id.is_nil() {
            delete_request_session(&state.request_id);
        }
        // Clean call mappings if tools were dispatched
        if !state.dispatched_tools.is_empty() {
            let call_ids: Vec<String> = state.dispatched_tools.iter().map(|t| t.id.clone()).collect();
            delete_call_sessions(&call_ids);
        }
    }

    /// Handle a cancel signal from the frontend.
    ///
    /// Cleans up any in-flight KV mappings and resets the turn to Idle.
    fn handle_cancel(session_id: &str) -> Result<(), SysError> {
        let mut state = TurnState::load(session_id);
        if state.phase == Phase::Idle {
            return Ok(());
        }
        let _ = sys::log("info", format!("Cancelling turn for session {session_id}"));
        Self::cleanup_inflight_mappings(&state);
        state.reset_turn();
        state.phase = Phase::Idle;
        state.save()
    }

    /// Publish an LLM generation request to the provider capsule.
    fn publish_llm_request(state: &TurnState, messages: &[Message]) -> Result<(), SysError> {
        let model =
            sys::get_config_string("model").unwrap_or_else(|_| "claude-sonnet-4-20250514".into());

        let tools = Self::load_tool_schemas();
        let llm_topic = Self::active_llm_topic();

        // Store request_id -> session_id mapping so handle_llm_stream
        // can resolve the owning session from the stream's request_id.
        store_request_session(&state.request_id, &state.session_id)?;

        if let Err(e) = ipc::publish_json(
            &llm_topic,
            &IpcPayload::LlmRequest {
                request_id: state.request_id,
                model,
                messages: messages.to_vec(),
                tools,
                system: state.system_prompt.clone(),
            },
        ) {
            delete_request_session(&state.request_id);
            return Err(e);
        }
        Ok(())
    }

    /// Fetch conversation history from the session capsule.
    fn fetch_messages(session_id: &str) -> Result<Vec<Message>, SysError> {
        Self::fetch_messages_inner(session_id, None)
    }

    /// Fetch conversation history with atomic append-before-read.
    ///
    /// The provided messages are appended to session storage and included
    /// in the returned history in a single atomic operation, eliminating
    /// the race between separate append + fetch calls.
    fn fetch_messages_with_append(
        session_id: &str,
        messages_to_append: &[Message],
    ) -> Result<Vec<Message>, SysError> {
        Self::fetch_messages_inner(session_id, Some(messages_to_append))
    }

    /// Core implementation for session message fetching.
    ///
    /// # Known limitation
    ///
    /// `session.response.get_messages` is a broadcast topic. If multiple
    /// react instances fetch concurrently, one may drain the other's
    /// response. The correlation ID check prevents misrouting, but the
    /// drained response is lost and the other instance times out. Fix:
    /// use request-scoped reply topics (`session.response.<correlation_id>`).
    ///
    /// # IPC envelope format
    ///
    /// The host delivers drain results as:
    /// ```json
    /// { "messages": [IpcMessage, ...], "dropped": N, "lagged": N }
    /// ```
    /// Each `IpcMessage` has `{ topic, payload, source_id, timestamp }`.
    /// The session capsule publishes raw JSON via `publish_json`, which
    /// the host wraps as `IpcPayload::Custom { data }`. So the actual
    /// response data lives at `envelope.messages[0].payload.data`.
    fn fetch_messages_inner(
        session_id: &str,
        append_before_read: Option<&[Message]>,
    ) -> Result<Vec<Message>, SysError> {
        let correlation_id = Uuid::new_v4().to_string();

        // Resolve timeout once (host call) rather than inside the closure.
        let timeout = session_timeout_ms();

        // Subscribe BEFORE publishing to avoid delivery race
        let handle = ipc::subscribe("session.response.get_messages")?;

        // Guard: ensure unsubscribe runs even on early return
        let result = (|| -> Result<Vec<Message>, SysError> {
            let mut request = serde_json::json!({
                "correlation_id": correlation_id,
                "session_id": session_id,
            });

            if let Some(msgs) = append_before_read {
                if !msgs.is_empty() {
                    request["append_before_read"] = serde_json::to_value(msgs).map_err(|e| {
                        SysError::ApiError(format!("Failed to serialize append messages: {e}"))
                    })?;
                }
            }

            ipc::publish_json("session.request.get_messages", &request)?;

            let response_bytes = ipc::recv_bytes(&handle, timeout).map_err(|e| {
                SysError::ApiError(format!(
                    "Session response timed out after {timeout}ms: {e}"
                ))
            })?;

            let envelope: serde_json::Value =
                serde_json::from_slice(&response_bytes).map_err(|e| {
                    SysError::ApiError(format!("Failed to parse session response: {e}"))
                })?;

            // Navigate the IPC drain envelope to find the session response.
            // Path: envelope.messages[0].payload.data.{correlation_id, messages}
            let ipc_messages = envelope
                .get("messages")
                .and_then(|m| m.as_array())
                .ok_or_else(|| {
                    SysError::ApiError("Session response envelope has no messages array".into())
                })?;

            // Empty envelope means the subscription timed out with no response.
            // The host returns Ok({"messages":[],...}) on timeout, not an error.
            if ipc_messages.is_empty() {
                return Err(SysError::ApiError(format!(
                    "Session capsule timed out - no response within {timeout}ms"
                )));
            }

            for msg in ipc_messages {
                let data = match msg.get("payload").and_then(|p| p.get("data")) {
                    Some(d) => d,
                    None => {
                        let _ = sys::log(
                            "debug",
                            "Skipping IPC message with no payload.data (not Custom type)",
                        );
                        continue;
                    },
                };

                let resp_correlation = data
                    .get("correlation_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if resp_correlation != correlation_id {
                    continue;
                }

                // Found our response. Extract messages.
                let messages: Vec<Message> = data
                    .get("messages")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| {
                        SysError::ApiError(format!("Failed to parse session messages: {e}"))
                    })?
                    .unwrap_or_default();

                return Ok(messages);
            }

            Err(SysError::ApiError(
                "Session response correlation ID not found in envelope".into(),
            ))
        })();

        // Always unsubscribe, regardless of success/failure
        let _ = ipc::unsubscribe(&handle);

        result
    }

    /// Resolve the active LLM provider topic from the registry.
    fn active_llm_topic() -> String {
        kv::get_bytes("llm_provider_topic")
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                sys::get_config_string("llm_provider_topic")
                    .unwrap_or_else(|_| "llm.request.generate.anthropic".into())
            })
    }

    /// Load tool schemas from KV.
    fn load_tool_schemas() -> Vec<LlmToolDefinition> {
        kv::get_json::<Vec<LlmToolDefinition>>("tool_schemas").unwrap_or_default()
    }
}
