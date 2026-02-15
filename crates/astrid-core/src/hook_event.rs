//! Hook event types shared across crates.
//!
//! `HookEvent` lives in `astrid-core` so that both `astrid-hooks` and
//! `astrid-plugins` can reference it without creating a circular dependency.

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
    /// A subagent is starting.
    SubagentStart,
    /// A subagent has stopped.
    SubagentStop,
    /// Gateway server is starting.
    GatewayStart,
    /// Gateway server is stopping.
    GatewayStop,
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
            Self::SubagentStart => write!(f, "subagent_start"),
            Self::SubagentStop => write!(f, "subagent_stop"),
            Self::GatewayStart => write!(f, "gateway_start"),
            Self::GatewayStop => write!(f, "gateway_stop"),
        }
    }
}
