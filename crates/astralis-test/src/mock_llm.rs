//! Mock LLM provider for testing.
//!
//! Provides [`MockLlmProvider`] — a deterministic, queue-based implementation of
//! [`LlmProvider`] that replays pre-configured turns. This enables integration tests
//! for agent loops, tool-call flows, and streaming consumers without hitting a real API.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream;
use serde_json::Value;
use uuid::Uuid;

use astralis_llm::{
    LlmError, LlmProvider, LlmResponse, LlmResult, LlmToolDefinition, Message, StopReason,
    StreamBox, StreamEvent, ToolCall, Usage,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single scripted turn that the mock provider will replay.
#[derive(Debug, Clone)]
pub enum MockLlmTurn {
    /// A text response.
    Text {
        /// The text content the assistant produces.
        text: String,
        /// Optional `(input_tokens, output_tokens)` usage override.
        usage: Option<(usize, usize)>,
    },
    /// One or more tool calls.
    ToolCalls {
        /// The tool calls to emit.
        calls: Vec<MockToolCall>,
        /// Optional `(input_tokens, output_tokens)` usage override.
        usage: Option<(usize, usize)>,
    },
    /// Produce an error.
    Error(
        /// The error message.
        String,
    ),
}

impl MockLlmTurn {
    /// Create a text turn with default usage.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            usage: None,
        }
    }

    /// Create a text turn with explicit usage.
    #[must_use]
    pub fn text_with_usage(text: impl Into<String>, input: usize, output: usize) -> Self {
        Self::Text {
            text: text.into(),
            usage: Some((input, output)),
        }
    }

    /// Create a tool-calls turn with default usage.
    #[must_use]
    pub fn tool_calls(calls: Vec<MockToolCall>) -> Self {
        Self::ToolCalls { calls, usage: None }
    }

    /// Create an error turn.
    #[must_use]
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Error(msg.into())
    }
}

/// A single tool call specification for [`MockLlmTurn::ToolCalls`].
#[derive(Debug, Clone)]
pub struct MockToolCall {
    /// Unique call ID.
    pub id: String,
    /// Tool name (e.g. `"read_file"`).
    pub name: String,
    /// JSON arguments for the call.
    pub arguments: Value,
}

impl MockToolCall {
    /// Create a new mock tool call with an auto-generated ID.
    #[must_use]
    pub fn new(name: impl Into<String>, args: Value) -> Self {
        Self {
            id: format!("mock-call-{}", Uuid::new_v4()),
            name: name.into(),
            arguments: args,
        }
    }

    /// Create a new mock tool call with an explicit ID.
    #[must_use]
    pub fn with_id(id: impl Into<String>, name: impl Into<String>, args: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }
    }
}

// ---------------------------------------------------------------------------
// MockLlmProvider
// ---------------------------------------------------------------------------

/// A deterministic, queue-based [`LlmProvider`] for tests.
///
/// Turns are popped from the front of the queue on each call to
/// [`stream`](LlmProvider::stream) or [`complete`](LlmProvider::complete).
/// If the queue is exhausted, an error is returned. After each call the
/// messages passed by the caller are captured and can be inspected via
/// [`captured_messages`](Self::captured_messages).
pub struct MockLlmProvider {
    turns: Mutex<VecDeque<MockLlmTurn>>,
    call_count: Mutex<usize>,
    captured_messages: Mutex<Vec<Vec<Message>>>,
}

impl MockLlmProvider {
    /// Create a new mock provider preloaded with the given turns.
    #[must_use]
    pub fn new(turns: Vec<MockLlmTurn>) -> Self {
        Self {
            turns: Mutex::new(VecDeque::from(turns)),
            call_count: Mutex::new(0),
            captured_messages: Mutex::new(Vec::new()),
        }
    }

    /// Return the number of times `stream` or `complete` has been called.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn call_count(&self) -> usize {
        *self.call_count.lock().expect("lock poisoned")
    }

    /// Return a snapshot of all captured message slices, one per call.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn captured_messages(&self) -> Vec<Vec<Message>> {
        self.captured_messages
            .lock()
            .expect("lock poisoned")
            .clone()
    }

    /// Record a call: bump counter, capture messages, pop next turn.
    fn next_turn(&self, messages: &[Message]) -> Result<MockLlmTurn, LlmError> {
        {
            let mut count = self.call_count.lock().expect("lock poisoned");
            *count = count.saturating_add(1);
        }
        {
            let mut captured = self.captured_messages.lock().expect("lock poisoned");
            captured.push(messages.to_vec());
        }

        let mut turns = self.turns.lock().expect("lock poisoned");
        turns.pop_front().ok_or_else(|| {
            LlmError::StreamingError("MockLlmProvider: no more turns queued".to_string())
        })
    }

    /// Default usage when none is specified.
    fn default_usage() -> (usize, usize) {
        (100, 50)
    }
}

#[async_trait]
#[allow(clippy::unnecessary_literal_bound)]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn model(&self) -> &str {
        "mock-model"
    }

    fn max_context_length(&self) -> usize {
        200_000
    }

    async fn stream(
        &self,
        messages: &[Message],
        _tools: &[LlmToolDefinition],
        _system: &str,
    ) -> LlmResult<StreamBox> {
        let turn = self.next_turn(messages)?;

        let events: Vec<LlmResult<StreamEvent>> = match turn {
            MockLlmTurn::Text { text, usage } => {
                let (inp, out) = usage.unwrap_or_else(Self::default_usage);
                vec![
                    Ok(StreamEvent::TextDelta(text)),
                    Ok(StreamEvent::Usage {
                        input_tokens: inp,
                        output_tokens: out,
                    }),
                    Ok(StreamEvent::Done),
                ]
            },
            MockLlmTurn::ToolCalls { calls, usage } => {
                let (inp, out) = usage.unwrap_or_else(Self::default_usage);
                let mut evts: Vec<LlmResult<StreamEvent>> = Vec::new();

                for call in &calls {
                    let args_json =
                        serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string());

                    evts.push(Ok(StreamEvent::ToolCallStart {
                        id: call.id.clone(),
                        name: call.name.clone(),
                    }));
                    evts.push(Ok(StreamEvent::ToolCallDelta {
                        id: call.id.clone(),
                        args_delta: args_json,
                    }));
                    evts.push(Ok(StreamEvent::ToolCallEnd {
                        id: call.id.clone(),
                    }));
                }

                evts.push(Ok(StreamEvent::Usage {
                    input_tokens: inp,
                    output_tokens: out,
                }));
                evts.push(Ok(StreamEvent::Done));
                evts
            },
            MockLlmTurn::Error(msg) => {
                vec![Ok(StreamEvent::Error(msg))]
            },
        };

        Ok(Box::pin(stream::iter(events)))
    }

    async fn complete(
        &self,
        messages: &[Message],
        _tools: &[LlmToolDefinition],
        _system: &str,
    ) -> LlmResult<LlmResponse> {
        let turn = self.next_turn(messages)?;

        match turn {
            MockLlmTurn::Text { text, usage } => {
                let (inp, out) = usage.unwrap_or_else(Self::default_usage);
                Ok(LlmResponse {
                    message: Message::assistant(text),
                    has_tool_calls: false,
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: inp,
                        output_tokens: out,
                    },
                })
            },
            MockLlmTurn::ToolCalls { calls, usage } => {
                let (inp, out) = usage.unwrap_or_else(Self::default_usage);
                let tool_calls: Vec<ToolCall> = calls
                    .into_iter()
                    .map(|c| ToolCall::new(c.id, c.name).with_arguments(c.arguments))
                    .collect();

                Ok(LlmResponse {
                    message: Message::assistant_with_tools(tool_calls),
                    has_tool_calls: true,
                    stop_reason: StopReason::ToolUse,
                    usage: Usage {
                        input_tokens: inp,
                        output_tokens: out,
                    },
                })
            },
            MockLlmTurn::Error(msg) => Err(LlmError::StreamingError(msg)),
        }
    }
}
