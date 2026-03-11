//! Hook event types shared across crates.
//!
//! `HookEvent` lives in `astrid-core` so that both `astrid-hooks` and
//! capsule crates can reference it without creating a circular dependency.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Events that can trigger hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Session has started.
    SessionStart,
    /// Session is ending.
    SessionEnd,
    /// User has submitted a prompt.
    UserPrompt,
    /// Before a tool call is executed.
    PreToolCall,
    /// After a tool call completes successfully.
    PostToolCall,
    /// A tool call resulted in an error.
    ToolError,
    /// Before an approval request is shown.
    PreApproval,
    /// After an approval decision is made.
    PostApproval,
    /// A notification needs to be sent.
    Notification,
    /// Before context compaction.
    PreCompact,
    /// After context compaction.
    PostCompact,
    /// Before the LLM prompt is assembled.
    PromptBuild,
    /// Before a response message is sent to the user.
    MessageSend,
    /// Before a session is reset.
    SessionReset,
    /// Before model/provider selection is resolved.
    ModelResolve,
    /// A user message has been received.
    MessageReceived,
    /// A response message has been delivered.
    MessageSent,
    /// The agent's cognitive loop has completed its run.
    AgentLoopEnd,
    /// A tool result is about to be persisted to history.
    ToolResultPersist,
    /// A subagent is starting.
    SubagentStart,
    /// A subagent has stopped.
    SubagentStop,
    /// Kernel daemon is starting.
    KernelStart,
    /// Kernel daemon is stopping.
    KernelStop,
}

impl fmt::Display for HookEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionStart => write!(f, "session_start"),
            Self::SessionEnd => write!(f, "session_end"),
            Self::UserPrompt => write!(f, "user_prompt"),
            Self::PreToolCall => write!(f, "pre_tool_call"),
            Self::PostToolCall => write!(f, "post_tool_call"),
            Self::ToolError => write!(f, "tool_error"),
            Self::PreApproval => write!(f, "pre_approval"),
            Self::PostApproval => write!(f, "post_approval"),
            Self::Notification => write!(f, "notification"),
            Self::PreCompact => write!(f, "pre_compact"),
            Self::PostCompact => write!(f, "post_compact"),
            Self::PromptBuild => write!(f, "prompt_build"),
            Self::MessageSend => write!(f, "message_send"),
            Self::SessionReset => write!(f, "session_reset"),
            Self::ModelResolve => write!(f, "model_resolve"),
            Self::MessageReceived => write!(f, "message_received"),
            Self::MessageSent => write!(f, "message_sent"),
            Self::AgentLoopEnd => write!(f, "agent_loop_end"),
            Self::ToolResultPersist => write!(f, "tool_result_persist"),
            Self::SubagentStart => write!(f, "subagent_start"),
            Self::SubagentStop => write!(f, "subagent_stop"),
            Self::KernelStart => write!(f, "kernel_start"),
            Self::KernelStop => write!(f, "kernel_stop"),
        }
    }
}
