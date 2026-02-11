//! TUI state machine — stripped to Nexus view only.

use std::time::{Duration, Instant};

// ─── Slash Command Palette ──────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub(crate) struct SlashCommandDef {
    pub name: &'static str,
    pub description: &'static str,
}

pub(crate) const PALETTE_MAX_VISIBLE: usize = 6;

pub(crate) const SLASH_COMMANDS: &[SlashCommandDef] = &[
    SlashCommandDef {
        name: "/help",
        description: "Show available commands",
    },
    SlashCommandDef {
        name: "/clear",
        description: "Clear conversation history",
    },
    SlashCommandDef {
        name: "/info",
        description: "Show daemon status",
    },
    SlashCommandDef {
        name: "/servers",
        description: "List MCP servers",
    },
    SlashCommandDef {
        name: "/tools",
        description: "List available tools",
    },
    SlashCommandDef {
        name: "/allowances",
        description: "Show active allowances",
    },
    SlashCommandDef {
        name: "/budget",
        description: "Show budget usage",
    },
    SlashCommandDef {
        name: "/audit",
        description: "Show recent audit entries",
    },
    SlashCommandDef {
        name: "/save",
        description: "Save current session",
    },
    SlashCommandDef {
        name: "/sessions",
        description: "List active sessions",
    },
];

// ─── UI State Machine ────────────────────────────────────────────

/// UI state machine states.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UiState {
    /// Waiting for user input.
    Idle,
    /// Agent is thinking/processing.
    Thinking { start_time: Instant, dots: usize },
    /// Awaiting approval for tool use.
    AwaitingApproval,
    /// Tool is currently running.
    ToolRunning {
        tool_name: String,
        start_time: Instant,
    },
    /// Streaming response from agent.
    Streaming { start_time: Instant },
    /// Error state.
    Error { message: String },
    /// Interrupted by user.
    Interrupted,
}

// ─── Messages ────────────────────────────────────────────────────

/// Message sender role.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MessageRole {
    User,
    Assistant,
    System,
}

/// Special message kinds for styled rendering.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) enum MessageKind {
    DiffHeader,
    DiffRemoved,
    DiffAdded,
    DiffFooter,
    /// Inline tool result (index into `completed_tools`).
    ToolResult(usize),
}

/// A conversation message.
#[derive(Debug, Clone)]
pub(crate) struct Message {
    pub role: MessageRole,
    pub content: String,
    #[allow(dead_code)]
    pub timestamp: Instant,
    pub kind: Option<MessageKind>,
    /// Whether to add a blank line after this message.
    pub spacing: bool,
}

/// A single entry in the Nexus unified timeline.
#[derive(Debug, Clone)]
pub(crate) enum NexusEntry {
    Message(Message),
}

// ─── Tool Status ─────────────────────────────────────────────────

/// Status of a tool execution.
#[derive(Debug, Clone)]
pub(crate) struct ToolStatus {
    /// The tool call ID from the LLM, used to match results to running tools.
    pub id: String,
    pub name: String,
    /// Primary argument for display, e.g. `"src/auth.rs"` for `read_file`.
    pub display_arg: String,
    pub status: ToolStatusKind,
    pub start_time: Instant,
    pub end_time: Option<Instant>,
    pub output: Option<String>,
    pub expanded: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) enum ToolStatusKind {
    Pending,
    Running,
    Success,
    Failed(String),
    Denied,
}

// ─── Approval ────────────────────────────────────────────────────

/// A pending approval request (TUI-local representation).
#[derive(Debug, Clone)]
pub(crate) struct ApprovalRequest {
    pub id: String,
    pub tool_name: String,
    pub description: String,
    pub risk_level: RiskLevel,
    pub details: Vec<(String, String)>,
}

/// Risk level for tool calls.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RiskLevel {
    Low,
    Medium,
    High,
}

// ─── Pending Actions ─────────────────────────────────────────────

/// Deferred action to send to the daemon.
#[derive(Debug, Clone)]
pub(crate) enum PendingAction {
    Approve {
        request_id: String,
        decision: ApprovalDecisionKind,
    },
    Deny {
        request_id: String,
        reason: Option<String>,
    },
    SendInput(String),
    CancelTurn,
}

/// What the user chose for approval.
#[derive(Debug, Clone)]
pub(crate) enum ApprovalDecisionKind {
    Once,
    Session,
    Always,
}

// ─── App ─────────────────────────────────────────────────────────

/// Main application state.
pub(crate) struct App {
    // ── UI state ──
    pub state: UiState,
    pub should_quit: bool,
    pub quit_pending: bool,

    // ── Content ──
    pub messages: Vec<Message>,
    pub nexus_stream: Vec<NexusEntry>,
    pub running_tools: Vec<ToolStatus>,
    pub completed_tools: Vec<ToolStatus>,
    pub pending_approvals: Vec<ApprovalRequest>,
    pub selected_approval: usize,

    // ── Input ──
    pub input: String,
    pub cursor_pos: usize,
    pub scroll_offset: usize,

    // ── Slash palette ──
    pub palette_selected: usize,
    pub palette_scroll_offset: usize,

    // ── Display ──
    pub working_dir: String,
    pub model_name: String,
    pub context_usage: f32,
    pub tokens_streamed: usize,
    pub session_id_short: String,

    // ── Timing ──
    pub last_completed: Option<(String, Duration)>,
    pub last_completed_at: Option<Instant>,
    pub stream_buffer: String,

    // ── Actions ──
    pub pending_actions: Vec<PendingAction>,
}

impl App {
    /// Create a new app instance.
    pub(crate) fn new(working_dir: String, model_name: String, session_id_short: String) -> Self {
        Self {
            state: UiState::Idle,
            should_quit: false,
            quit_pending: false,

            messages: Vec::new(),
            nexus_stream: Vec::new(),
            running_tools: Vec::new(),
            completed_tools: Vec::new(),
            pending_approvals: Vec::new(),
            selected_approval: 0,

            input: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,

            palette_selected: 0,
            palette_scroll_offset: 0,

            working_dir,
            model_name,
            context_usage: 0.0,
            tokens_streamed: 0,
            session_id_short,

            last_completed: None,
            last_completed_at: None,
            stream_buffer: String::new(),

            pending_actions: Vec::new(),
        }
    }

    /// Whether the slash command palette should be displayed.
    pub(crate) fn palette_active(&self) -> bool {
        matches!(self.state, UiState::Idle | UiState::Interrupted) && self.input.starts_with('/')
    }

    /// Return the filtered list of slash commands matching the current input prefix.
    pub(crate) fn palette_filtered(&self) -> Vec<&'static SlashCommandDef> {
        let prefix = &self.input;
        SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.name.starts_with(prefix))
            .collect()
    }

    /// Reset palette selection and scroll to top.
    pub(crate) fn palette_reset(&mut self) {
        self.palette_selected = 0;
        self.palette_scroll_offset = 0;
    }

    /// Submit the current input, returning the text if non-empty.
    pub(crate) fn submit_input(&mut self) -> Option<String> {
        if self.input.trim().is_empty() {
            return None;
        }
        let content = std::mem::take(&mut self.input);
        self.cursor_pos = 0;
        Some(content)
    }

    /// Push a user/assistant/system message and add it to the nexus stream.
    pub(crate) fn push_message(&mut self, role: MessageRole, content: String) {
        let msg = Message {
            role,
            content,
            timestamp: Instant::now(),
            kind: None,
            spacing: true,
        };
        self.nexus_stream.push(NexusEntry::Message(msg.clone()));
        self.messages.push(msg);
    }

    /// Push a system notice.
    pub(crate) fn push_notice(&mut self, text: &str) {
        self.push_message(MessageRole::System, text.to_string());
    }

    /// Approve a pending tool call.
    ///
    /// Does NOT push to `running_tools` — the daemon will send a
    /// `ToolCallStart` event once the tool actually begins executing,
    /// which avoids duplicate entries.
    pub(crate) fn approve_tool(&mut self, id: &str, decision: ApprovalDecisionKind) {
        if let Some(pos) = self.pending_approvals.iter().position(|a| a.id == id) {
            let approval = self.pending_approvals.remove(pos);

            self.pending_actions.push(PendingAction::Approve {
                request_id: id.to_string(),
                decision,
            });

            // Transition to ToolRunning or Thinking while waiting for daemon.
            if let Some(tool) = self.running_tools.last() {
                self.state = UiState::ToolRunning {
                    tool_name: tool.name.clone(),
                    start_time: tool.start_time,
                };
            } else {
                self.state = UiState::Thinking {
                    start_time: Instant::now(),
                    dots: 0,
                };
            }

            let _ = approval; // consumed above
        }

        if self.pending_approvals.is_empty() && self.running_tools.is_empty() {
            // Still waiting for the daemon to resume after approval.
            self.state = UiState::Thinking {
                start_time: Instant::now(),
                dots: 0,
            };
        } else if !self.pending_approvals.is_empty() {
            self.state = UiState::AwaitingApproval;
        }
    }

    /// Deny a pending tool call.
    pub(crate) fn deny_tool(&mut self, id: &str) {
        self.pending_approvals.retain(|a| a.id != id);

        self.pending_actions.push(PendingAction::Deny {
            request_id: id.to_string(),
            reason: None,
        });

        self.push_notice("Tool call denied.");

        if self.pending_approvals.is_empty() {
            self.state = UiState::Idle;
        }
    }
}
