//! Demo player - executes scripted demo scenarios.

use super::scenarios::{
    AgentStatusDemo, ApprovalChoice, AuditOutcomeDemo, DemoScenario, DemoStep, FileStatus,
    HealthStatusDemo, NexusCategoryDemo, SidebarState, TaskStatus, ThreatLevelDemo, ToolRisk, View,
};
use crate::ui::state::{
    ActivityEvent, AgentSnapshot, AgentStatus, App, ApprovalRequest, ApprovalSnapshot,
    AuditOutcome, AuditSnapshot, CapabilitySnapshot, ChainIntegrity, DenialRecord, EventCategory,
    EventRecord, FileEntry, FileEntryStatus, HealthCheck, HealthStatus, Message, MessageKind,
    MessageRole, NexusCategory, NexusEntry, OverallHealth, RiskLevel, SidebarMode, SubAgentNode,
    SubAgentStatus, Task, TaskColumn, ThreatLevel, ToolStatus, ToolStatusKind, UiState, ViewMode,
};
use std::time::{Duration, Instant};

/// Plays a demo scenario step by step
pub(crate) struct DemoPlayer {
    scenario: DemoScenario,
    current_step: usize,
    step_start: Instant,
    /// For `UserTypes` - which character we're on
    typing_index: usize,
    /// For `AgentStreams` - which word we're on
    streaming_index: usize,
    /// Paused waiting for step to complete
    waiting: bool,
    /// Fast-forward mode - skip timing delays
    pub fast_forward: bool,
    /// Counter for generating unique sub-agent IDs
    subagent_counter: usize,
    /// Counter for generating unique audit entry IDs
    audit_counter: usize,
}

impl DemoPlayer {
    pub(crate) fn new(scenario: DemoScenario) -> Self {
        Self {
            scenario,
            current_step: 0,
            step_start: Instant::now(),
            typing_index: 0,
            streaming_index: 0,
            waiting: false,
            fast_forward: false,
            subagent_counter: 0,
            audit_counter: 0,
        }
    }

    /// Create a new player in fast-forward mode (for snapshots)
    pub(crate) fn new_fast_forward(scenario: DemoScenario) -> Self {
        Self {
            scenario,
            current_step: 0,
            step_start: Instant::now(),
            typing_index: 0,
            streaming_index: 0,
            waiting: false,
            fast_forward: true,
            subagent_counter: 0,
            audit_counter: 0,
        }
    }

    /// Advance the demo, returning true if demo is complete
    #[allow(
        clippy::too_many_lines,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub(crate) fn advance(&mut self, app: &mut App) -> bool {
        if self.current_step >= self.scenario.steps.len() {
            return true; // Demo complete
        }

        let step = &self.scenario.steps[self.current_step];
        let elapsed = self.step_start.elapsed();

        match step {
            DemoStep::Pause(duration) => {
                if self.fast_forward || elapsed >= *duration {
                    self.next_step();
                }
            },

            DemoStep::UserTypes {
                text,
                typing_speed_ms,
            } => {
                if self.fast_forward {
                    // Instantly complete typing
                    app.input.clone_from(text);
                    app.cursor_pos = text.len();
                    self.next_step();
                } else {
                    let chars_to_show = if *typing_speed_ms == 0 {
                        text.len()
                    } else {
                        #[allow(clippy::arithmetic_side_effects)] // divisor checked non-zero above
                        let c = (elapsed.as_millis() / u128::from(*typing_speed_ms)) as usize;
                        c
                    };
                    if chars_to_show > self.typing_index && self.typing_index < text.len() {
                        // Add next character to input
                        let next_char = text.chars().nth(self.typing_index).unwrap();
                        app.input.push(next_char);
                        app.cursor_pos = app.input.len();
                        self.typing_index = self.typing_index.saturating_add(1);
                    }
                    if self.typing_index >= text.len() {
                        self.next_step();
                    }
                }
            },

            DemoStep::UserSubmits => {
                // Add message and clear input
                if !app.input.is_empty() {
                    let msg = Message {
                        role: MessageRole::User,
                        content: std::mem::take(&mut app.input),
                        timestamp: Instant::now(),
                        kind: None,
                        spacing: true,
                    };
                    app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                    app.messages.push(msg);
                    app.cursor_pos = 0;
                }
                self.next_step();
            },

            DemoStep::AgentThinking { duration } => {
                if self.fast_forward {
                    // Skip thinking animation
                    self.next_step();
                } else {
                    if !self.waiting {
                        app.state = UiState::Thinking {
                            start_time: Instant::now(),
                            dots: 0,
                        };
                        app.tokens_streamed = 0;
                        self.waiting = true;
                    }

                    // Animate token counter during thinking (shows input processing)
                    let progress =
                        (elapsed.as_millis() as f32 / duration.as_millis() as f32).min(1.0);
                    let input_tokens = 50; // Simulated input token count
                    app.tokens_streamed = (input_tokens as f32 * progress) as usize;

                    if elapsed >= *duration {
                        // Record completed activity for status bar
                        // Safety: division and modulo cannot overflow
                        #[allow(clippy::arithmetic_side_effects)]
                        let verb_index =
                            (duration.as_millis() / 3000) as usize % crate::ui::FUN_VERBS.len();
                        let (_, past) = crate::ui::FUN_VERBS[verb_index];
                        app.last_completed = Some((past.to_string(), *duration));
                        app.last_completed_at = Some(Instant::now());

                        app.state = UiState::Idle;
                        self.waiting = false;
                        self.next_step();
                    }
                }
            },

            DemoStep::AgentStreams {
                text,
                word_delay_ms,
            } => {
                // Calculate total tokens for this response
                let words: Vec<&str> = text.split_whitespace().collect();
                #[allow(clippy::arithmetic_side_effects)] // f32 mul for estimation
                let total_tokens = (words.len() as f32 * 1.3) as usize;
                let total_duration_ms = (*word_delay_ms as usize).saturating_mul(words.len());

                if self.fast_forward {
                    // Instantly add full message with final token count
                    let msg = Message {
                        role: MessageRole::Assistant,
                        content: text.clone(),
                        timestamp: Instant::now(),
                        kind: None,
                        spacing: true,
                    };
                    app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                    app.messages.push(msg);
                    app.tokens_streamed = total_tokens;
                    app.context_usage = (app.context_usage + 0.01).min(0.95);
                    self.next_step();
                } else {
                    if !self.waiting {
                        // Show full message immediately (no word-by-word streaming)
                        let msg = Message {
                            role: MessageRole::Assistant,
                            content: text.clone(),
                            timestamp: Instant::now(),
                            kind: None,
                            spacing: true,
                        };
                        app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                        app.messages.push(msg);
                        app.tokens_streamed = 0;
                        app.state = UiState::Streaming {
                            start_time: Instant::now(),
                        };
                        self.waiting = true;
                    }

                    // Animate token counter over the duration (shows activity)
                    let progress = (elapsed.as_millis() as f32 / total_duration_ms as f32).min(1.0);
                    app.tokens_streamed = (total_tokens as f32 * progress) as usize;
                    app.context_usage = (app.context_usage + 0.0005).min(0.95);

                    // Complete when duration elapsed
                    if elapsed.as_millis() >= total_duration_ms as u128 {
                        app.tokens_streamed = total_tokens;
                        app.last_completed = Some(("Responded".to_string(), elapsed));
                        app.last_completed_at = Some(Instant::now());
                        app.state = UiState::Idle;
                        self.waiting = false;
                        self.next_step();
                    }
                }
            },

            DemoStep::ToolRequest {
                name,
                description,
                args,
                risk,
            } => {
                if !self.waiting {
                    let risk_level = match risk {
                        ToolRisk::Low => RiskLevel::Low,
                        ToolRisk::Medium => RiskLevel::Medium,
                        ToolRisk::High => RiskLevel::High,
                    };
                    app.pending_approvals.push(ApprovalRequest {
                        id: app.pending_approvals.len(),
                        tool_name: name.clone(),
                        description: description.clone(),
                        risk_level,
                        details: args.clone(),
                    });
                    app.state = UiState::AwaitingApproval;
                    self.waiting = true;
                }
                // Wait here - next step (UserApproves) will clear
                self.next_step();
            },

            DemoStep::UserApproves { choice } => {
                if let Some(approval) = app.pending_approvals.pop() {
                    // Extract primary arg for display
                    let display_arg = approval
                        .details
                        .first()
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();

                    if matches!(choice, ApprovalChoice::Deny) {
                        // Add as denied tool (red ⏺ in message stream)
                        app.completed_tools.push(ToolStatus {
                            name: approval.tool_name,
                            display_arg,
                            status: ToolStatusKind::Denied,
                            start_time: Instant::now(),
                            end_time: Some(Instant::now()),
                            output: None,
                            expanded: false,
                        });

                        // Push inline tool result message
                        // Safety: we just pushed to completed_tools, so len() > 0
                        #[allow(clippy::arithmetic_side_effects)]
                        let tool_idx = app.completed_tools.len() - 1;
                        let msg = Message {
                            role: MessageRole::System,
                            content: String::new(),
                            timestamp: Instant::now(),
                            kind: Some(MessageKind::ToolResult(tool_idx)),
                            spacing: true,
                        };
                        app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                        app.messages.push(msg);
                        app.state = UiState::Idle;
                    } else {
                        // Move to running
                        app.running_tools.push(ToolStatus {
                            name: approval.tool_name,
                            display_arg,
                            status: ToolStatusKind::Running,
                            start_time: Instant::now(),
                            end_time: None,
                            output: None,
                            expanded: false,
                        });
                        app.state = UiState::ToolRunning {
                            tool_name: app.running_tools.last().unwrap().name.clone(),
                            start_time: Instant::now(),
                        };
                    }
                }
                self.waiting = false;
                self.next_step();
            },

            DemoStep::ToolExecutes {
                duration,
                output,
                success,
            } => {
                if !self.waiting {
                    self.waiting = true;
                }

                if self.fast_forward || elapsed >= *duration {
                    // Complete the tool
                    if let Some(mut completed) = app.running_tools.pop() {
                        if *success {
                            completed.status = ToolStatusKind::Success;
                            completed.output.clone_from(output);
                        } else {
                            completed.status =
                                ToolStatusKind::Failed(output.clone().unwrap_or_default());
                            // Don't duplicate error in output field
                            completed.output = None;
                        }
                        completed.end_time = Some(Instant::now());
                        app.last_completed = Some((format!("Ran {}", completed.name), *duration));
                        app.completed_tools.push(completed);

                        // Push inline tool result message
                        // Safety: we just pushed to completed_tools, so len() > 0
                        #[allow(clippy::arithmetic_side_effects)]
                        let tool_idx = app.completed_tools.len() - 1;
                        let msg = Message {
                            role: MessageRole::System,
                            content: String::new(),
                            timestamp: Instant::now(),
                            kind: Some(MessageKind::ToolResult(tool_idx)),
                            spacing: true,
                        };
                        app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                        app.messages.push(msg);
                    }
                    app.last_completed_at = Some(Instant::now());
                    app.state = UiState::Idle;
                    self.waiting = false;
                    self.next_step();
                }
            },

            DemoStep::SystemMessage(msg) => {
                let m = Message {
                    role: MessageRole::System,
                    content: msg.clone(),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false, // System messages are typically grouped
                };
                app.nexus_stream.push(NexusEntry::Message(m.clone()));
                app.messages.push(m);
                self.next_step();
            },

            DemoStep::Clear => {
                app.messages.clear();
                app.completed_tools.clear();
                app.pending_approvals.clear();
                app.running_tools.clear();
                app.files.clear();
                app.events.clear();
                app.nexus_stream.clear();
                self.next_step();
            },

            // ─── New UI Features ───────────────────────────────────────────
            DemoStep::BootSequence {
                cinematic,
                checks: _,
            } => {
                let cinematic = *cinematic;

                if self.fast_forward {
                    app.messages.clear();
                    app.context_usage = 0.02;
                    app.model_name = "claude-opus-4.6".to_string();
                    app.working_dir = "~/projects/astrid-demo".to_string();
                    self.next_step();
                } else if !self.waiting {
                    app.messages.clear();
                    app.context_usage = 0.02;
                    app.model_name = "claude-opus-4.6".to_string();
                    app.working_dir = "~/projects/astrid-demo".to_string();

                    if cinematic {
                        app.welcome_visible = true;
                    }
                    self.waiting = true;
                }

                let hold_time: u128 = if cinematic { 3000 } else { 1200 };
                if elapsed.as_millis() >= hold_time {
                    app.welcome_visible = false;
                    self.waiting = false;
                    self.next_step();
                }
            },

            DemoStep::SwitchView(view) => {
                let (view_mode, view_name) = match view {
                    View::Nexus => (ViewMode::Nexus, "Nexus"),
                    View::Missions => (ViewMode::Missions, "Missions"),
                    View::Stellar => (ViewMode::Stellar, "Atlas"),
                    View::Command => (ViewMode::Command, "Command"),
                    View::Topology => (ViewMode::Topology, "Topology"),
                    View::Shield => (ViewMode::Shield, "Shield"),
                    View::Chain => (ViewMode::Chain, "Chain"),
                    View::Pulse => (ViewMode::Pulse, "Pulse"),
                    View::Log => (ViewMode::Log, "Console"),
                };
                app.view = view_mode;

                if matches!(view, View::Log) {
                    app.sidebar = SidebarMode::Hidden;
                }

                let msg = Message {
                    role: MessageRole::System,
                    content: format!("─── {view_name} ───"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                };
                app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                app.messages.push(msg);
                self.next_step();
            },

            DemoStep::ToggleSidebar(state) => {
                app.sidebar = match state {
                    SidebarState::Expanded => SidebarMode::Expanded,
                    SidebarState::Collapsed => SidebarMode::Collapsed,
                    SidebarState::Hidden => SidebarMode::Hidden,
                };
                self.next_step();
            },

            DemoStep::OrbitStatus(status) => {
                app.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("━━ {status} ━━"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
                self.next_step();
            },

            DemoStep::AddTask { id, title, status } => {
                let icon = match status {
                    TaskStatus::Backlog => "○",
                    TaskStatus::Active => "◐",
                    TaskStatus::Review => "✧",
                    TaskStatus::Complete => "★",
                    TaskStatus::Blocked => "✦",
                };

                let column = match status {
                    TaskStatus::Backlog => TaskColumn::Backlog,
                    TaskStatus::Active => TaskColumn::Active,
                    TaskStatus::Review => TaskColumn::Review,
                    TaskStatus::Complete => TaskColumn::Complete,
                    TaskStatus::Blocked => TaskColumn::Blocked,
                };

                app.tasks.push(Task {
                    id: id.clone(),
                    title: title.clone(),
                    column,
                    agent_name: None,
                });

                app.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("  {icon} Task: {title}"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false,
                });
                self.next_step();
            },

            DemoStep::MoveTask { id, status } => {
                let status_name = match status {
                    TaskStatus::Backlog => "Backlog",
                    TaskStatus::Active => "Active",
                    TaskStatus::Review => "Review",
                    TaskStatus::Complete => "Complete",
                    TaskStatus::Blocked => "Blocked",
                };

                let column = match status {
                    TaskStatus::Backlog => TaskColumn::Backlog,
                    TaskStatus::Active => TaskColumn::Active,
                    TaskStatus::Review => TaskColumn::Review,
                    TaskStatus::Complete => TaskColumn::Complete,
                    TaskStatus::Blocked => TaskColumn::Blocked,
                };

                if let Some(task) = app.tasks.iter_mut().find(|t| t.id == *id) {
                    task.column = column;
                }

                app.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("  → Task moved to {status_name}"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false,
                });
                self.next_step();
            },

            DemoStep::ShowFile { path, status } => {
                let icon = match status {
                    FileStatus::Unchanged => "○",
                    FileStatus::Modified => "★",
                    FileStatus::Added => "+",
                    FileStatus::Deleted => "-",
                    FileStatus::Editing => "◐",
                    FileStatus::NeedsAttention => "✦",
                };

                let entry_status = match status {
                    FileStatus::Unchanged => FileEntryStatus::Unchanged,
                    FileStatus::Modified | FileStatus::NeedsAttention => FileEntryStatus::Modified,
                    FileStatus::Added => FileEntryStatus::Added,
                    FileStatus::Deleted => FileEntryStatus::Deleted,
                    FileStatus::Editing => FileEntryStatus::Editing,
                };
                let depth = path.matches('/').count();
                app.files.push(FileEntry {
                    path: path.clone(),
                    status: entry_status,
                    depth,
                    is_dir: false,
                });

                app.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("  {icon} {path}"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false,
                });
                self.next_step();
            },

            DemoStep::StreamEvent { icon, message } => {
                let category = if message.contains("approved") || message.contains("Approve") {
                    EventCategory::Approval
                } else if message.contains("Error") || message.contains("error") {
                    EventCategory::Error
                } else if message.contains("Session")
                    || message.contains("complete")
                    || message.contains("Complete")
                {
                    EventCategory::Session
                } else {
                    EventCategory::Tool
                };

                app.events.push(ActivityEvent {
                    timestamp: Instant::now(),
                    icon: icon.clone(),
                    message: message.clone(),
                    category,
                    agent_name: None,
                });

                let msg = Message {
                    role: MessageRole::System,
                    content: format!("  {icon} {message}"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false,
                };
                app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                app.messages.push(msg);
                self.next_step();
            },

            DemoStep::ShowDiff {
                file,
                removed,
                added,
            } => {
                let header = Message {
                    role: MessageRole::System,
                    content: format!("  ╭─ diff: {file} ─────────────────────"),
                    timestamp: Instant::now(),
                    kind: Some(MessageKind::DiffHeader),
                    spacing: false,
                };
                app.nexus_stream.push(NexusEntry::Message(header.clone()));
                app.messages.push(header);
                let mut line_num: usize = 1;
                for line in removed {
                    let m = Message {
                        role: MessageRole::System,
                        content: format!("  │ {line_num:>3} - {line}"),
                        timestamp: Instant::now(),
                        kind: Some(MessageKind::DiffRemoved),
                        spacing: false,
                    };
                    app.nexus_stream.push(NexusEntry::Message(m.clone()));
                    app.messages.push(m);
                    line_num = line_num.saturating_add(1);
                }
                for line in added {
                    let m = Message {
                        role: MessageRole::System,
                        content: format!("  │ {line_num:>3} + {line}"),
                        timestamp: Instant::now(),
                        kind: Some(MessageKind::DiffAdded),
                        spacing: false,
                    };
                    app.nexus_stream.push(NexusEntry::Message(m.clone()));
                    app.messages.push(m);
                    line_num = line_num.saturating_add(1);
                }
                let footer = Message {
                    role: MessageRole::System,
                    content: "  ╰───────────────────────────────────".to_string(),
                    timestamp: Instant::now(),
                    kind: Some(MessageKind::DiffFooter),
                    spacing: true,
                };
                app.nexus_stream.push(NexusEntry::Message(footer.clone()));
                app.messages.push(footer);
                self.next_step();
            },

            DemoStep::Narrate(text) => {
                app.nexus_stream.push(NexusEntry::SystemNotice {
                    timestamp: Instant::now(),
                    content: format!("═══ {text} ═══"),
                });

                app.messages.push(Message {
                    role: MessageRole::System,
                    content: String::new(),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false,
                });
                app.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("═══ {text} ═══"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false,
                });
                app.messages.push(Message {
                    role: MessageRole::System,
                    content: String::new(),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
                self.next_step();
            },

            // ─── Multi-Agent Demo Steps ───────────────────────────────────
            DemoStep::SpawnAgent { name, model } => {
                app.agents.push(AgentSnapshot {
                    name: name.clone(),
                    status: AgentStatus::Starting,
                    last_activity: Instant::now(),
                    current_activity: Some("Initializing...".to_string()),
                    current_tool: None,
                    request_count: 0,
                    last_error: None,
                    context_usage: 0.02,
                    budget_spent: 0.0,
                    active_subagents: 0,
                    pending_approvals: 0,
                    tokens_used: 0,
                });

                app.event_stream.push_back(EventRecord {
                    timestamp: Instant::now(),
                    agent_name: name.clone(),
                    event_type: "AgentSpawned".to_string(),
                    detail: format!("model={model}"),
                    category: EventCategory::Runtime,
                });

                app.nexus_stream.push(NexusEntry::AgentSpawned {
                    timestamp: Instant::now(),
                    name: name.clone(),
                    model: model.clone(),
                });

                app.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("  ✧ Agent spawned: {name} ({model})"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: false,
                });
                self.next_step();
            },

            DemoStep::SetAgentStatus { agent, status } => {
                let new_status = match status {
                    AgentStatusDemo::Ready => AgentStatus::Ready,
                    AgentStatusDemo::Busy => AgentStatus::Busy,
                    AgentStatusDemo::Paused => AgentStatus::Paused,
                    AgentStatusDemo::Error => AgentStatus::Error,
                    AgentStatusDemo::Starting => AgentStatus::Starting,
                };

                if let Some(a) = app.agents.iter_mut().find(|a| a.name == *agent) {
                    a.status = new_status;
                    a.last_activity = Instant::now();
                    if new_status == AgentStatus::Error {
                        a.last_error = Some("Operation failed".to_string());
                    }
                    if new_status == AgentStatus::Busy {
                        a.current_activity = Some("Processing...".to_string());
                    }
                }
                self.next_step();
            },

            DemoStep::SpawnSubAgent { parent_agent, task } => {
                self.subagent_counter = self.subagent_counter.saturating_add(1);
                let id = format!("sub-{:03}", self.subagent_counter);

                // Count depth based on parent
                let depth = app
                    .subagent_tree
                    .iter()
                    .filter(|n| n.parent_agent == *parent_agent)
                    .count();

                app.subagent_tree.push(SubAgentNode {
                    id: id.clone(),
                    parent_agent: parent_agent.clone(),
                    parent_subagent: None,
                    task: task.clone(),
                    status: SubAgentStatus::Running,
                    depth: depth.min(3),
                    started_at: Instant::now(),
                    duration: None,
                    expanded: true,
                });

                // Update parent agent's sub-agent count
                if let Some(a) = app.agents.iter_mut().find(|a| a.name == *parent_agent) {
                    a.active_subagents = a.active_subagents.saturating_add(1);
                }

                app.event_stream.push_back(EventRecord {
                    timestamp: Instant::now(),
                    agent_name: parent_agent.clone(),
                    event_type: "SubAgentSpawned".to_string(),
                    detail: format!("{id}: {task}"),
                    category: EventCategory::Runtime,
                });

                app.nexus_stream.push(NexusEntry::SubAgentLifecycle {
                    timestamp: Instant::now(),
                    agent: parent_agent.clone(),
                    subagent_id: id,
                    action: format!("spawned: {task}"),
                    status: SubAgentStatus::Running,
                });

                self.next_step();
            },

            DemoStep::CompleteSubAgent { id, success } => {
                let status = if *success {
                    SubAgentStatus::Completed
                } else {
                    SubAgentStatus::Failed
                };
                let action_str = if *success { "completed" } else { "failed" };

                if let Some(node) = app.subagent_tree.iter_mut().find(|n| n.id == *id) {
                    node.status = status;
                    node.duration = Some(node.started_at.elapsed());

                    let parent = node.parent_agent.clone();

                    app.nexus_stream.push(NexusEntry::SubAgentLifecycle {
                        timestamp: Instant::now(),
                        agent: parent.clone(),
                        subagent_id: id.clone(),
                        action: action_str.to_string(),
                        status,
                    });

                    // Update parent's count
                    if let Some(a) = app.agents.iter_mut().find(|a| a.name == parent) {
                        a.active_subagents = a.active_subagents.saturating_sub(1);
                    }
                }
                self.next_step();
            },

            DemoStep::GrantCapability {
                agent,
                resource,
                scope,
                ttl_secs,
            } => {
                let expires_in = ttl_secs.map(Duration::from_secs);
                app.active_capabilities.push(CapabilitySnapshot {
                    id: format!("cap-{}", app.active_capabilities.len().saturating_add(1)),
                    resource: resource.clone(),
                    permissions: vec!["execute".to_string()],
                    scope: scope.clone(),
                    expires_in,
                    use_count: 0,
                    agent_name: agent.clone(),
                });

                let event = EventRecord {
                    timestamp: Instant::now(),
                    agent_name: agent.clone(),
                    event_type: "CapabilityGranted".to_string(),
                    detail: format!("{resource} ({scope})"),
                    category: EventCategory::Security,
                };
                app.nexus_stream.push(NexusEntry::Event(event.clone()));
                app.event_stream.push_back(event);

                self.next_step();
            },

            DemoStep::SecurityViolation { agent, detail } => {
                app.recent_denials.push(DenialRecord {
                    agent_name: agent.clone(),
                    tool_name: detail.clone(),
                    risk_level: RiskLevel::High,
                    timestamp: Instant::now(),
                });

                app.event_stream.push_back(EventRecord {
                    timestamp: Instant::now(),
                    agent_name: agent.clone(),
                    event_type: "SecurityViolation".to_string(),
                    detail: detail.clone(),
                    category: EventCategory::Security,
                });

                app.nexus_stream.push(NexusEntry::SecurityAlert {
                    timestamp: Instant::now(),
                    agent: agent.clone(),
                    detail: detail.clone(),
                    level: app.threat_level,
                });

                self.next_step();
            },

            DemoStep::AddAuditEntry {
                agent,
                action,
                outcome,
            } => {
                self.audit_counter = self.audit_counter.saturating_add(1);
                let audit_outcome = match outcome {
                    AuditOutcomeDemo::Success => AuditOutcome::Success,
                    AuditOutcomeDemo::Failure => AuditOutcome::Failure,
                    AuditOutcomeDemo::Denied => AuditOutcome::Denied,
                    AuditOutcomeDemo::Violation => AuditOutcome::Violation,
                };

                // Generate a fake hash
                let hash = format!(
                    "{:08x}{:08x}",
                    self.audit_counter.wrapping_mul(0x9e37_79b9),
                    self.audit_counter.wrapping_mul(0x517c_c1b7)
                );

                let audit = AuditSnapshot {
                    id: self.audit_counter,
                    timestamp: Instant::now(),
                    agent_name: agent.clone(),
                    action: action.clone(),
                    auth_method: "Capability".to_string(),
                    outcome: audit_outcome,
                    detail: format!("{action} by {agent}"),
                    hash,
                };
                app.nexus_stream.push(NexusEntry::AuditEntry(audit.clone()));
                app.audit_entries.push_back(audit);

                // Update chain integrity
                app.chain_integrity = ChainIntegrity {
                    verified: true,
                    total_entries: app.audit_entries.len(),
                    break_at: None,
                };

                self.next_step();
            },

            DemoStep::SetBudget { agent, spent } => {
                app.budget.per_agent.insert(agent.clone(), *spent);
                app.budget.total_spent = app.budget.per_agent.values().sum();
                app.budget.burn_rate_per_hour = app.budget.total_spent * 0.5; // Simulated
                app.budget.input_tokens = (app.budget.total_spent * 5000.0) as usize;
                app.budget.output_tokens = (app.budget.total_spent * 1500.0) as usize;

                // Update agent budget
                if let Some(a) = app.agents.iter_mut().find(|a| a.name == *agent) {
                    a.budget_spent = *spent;
                    a.tokens_used = (*spent * 5000.0) as usize;
                }

                self.next_step();
            },

            DemoStep::SetHealth { component, status } => {
                let health_status = match status {
                    HealthStatusDemo::Ok => HealthStatus::Ok,
                    HealthStatusDemo::Degraded => HealthStatus::Degraded,
                    HealthStatusDemo::Down => HealthStatus::Down,
                };

                // Update or add
                if let Some(check) = app
                    .health
                    .checks
                    .iter_mut()
                    .find(|c| c.component == *component)
                {
                    check.status = health_status;
                } else {
                    let latency = match health_status {
                        HealthStatus::Ok => 1.0 + (app.health.checks.len() as f64 * 0.5),
                        HealthStatus::Degraded => 150.0,
                        HealthStatus::Down => 0.0,
                    };
                    app.health.checks.push(HealthCheck {
                        component: component.clone(),
                        status: health_status,
                        latency_ms: latency,
                    });
                }

                // Compute overall health
                let has_down = app
                    .health
                    .checks
                    .iter()
                    .any(|c| c.status == HealthStatus::Down);
                let has_degraded = app
                    .health
                    .checks
                    .iter()
                    .any(|c| c.status == HealthStatus::Degraded);

                app.health.overall = if has_down {
                    OverallHealth::Unhealthy
                } else if has_degraded {
                    OverallHealth::Degraded
                } else {
                    OverallHealth::Healthy
                };

                // Set some performance metrics
                app.performance.avg_tool_latency_ms = 245.0;
                app.performance.avg_llm_latency_ms = 2100.0;
                app.performance.avg_approval_wait_ms = 8400.0;
                app.performance.tool_calls_per_min = 4.2;
                app.performance.events_per_min = 12.3;
                app.gateway_uptime = Duration::from_secs(8072);

                self.next_step();
            },

            DemoStep::SetThreatLevel(level) => {
                app.threat_level = match level {
                    ThreatLevelDemo::Low => ThreatLevel::Low,
                    ThreatLevelDemo::Elevated => ThreatLevel::Elevated,
                    ThreatLevelDemo::High => ThreatLevel::High,
                    ThreatLevelDemo::Critical => ThreatLevel::Critical,
                };
                self.next_step();
            },

            DemoStep::AddShieldApproval { agent, tool, risk } => {
                let risk_level = match risk {
                    ToolRisk::Low => RiskLevel::Low,
                    ToolRisk::Medium => RiskLevel::Medium,
                    ToolRisk::High => RiskLevel::High,
                };

                let approval = ApprovalSnapshot {
                    id: app.shield_approvals.len(),
                    agent_name: agent.clone(),
                    tool_name: tool.clone(),
                    risk_level,
                    description: tool.clone(),
                    timestamp: Instant::now(),
                };
                app.nexus_stream
                    .push(NexusEntry::Approval(approval.clone()));
                app.shield_approvals.push(approval);

                // Update agent's pending count
                if let Some(a) = app.agents.iter_mut().find(|a| a.name == *agent) {
                    a.pending_approvals = a.pending_approvals.saturating_add(1);
                }

                self.next_step();
            },

            DemoStep::AddEventRecord {
                agent,
                event_type,
                detail,
            } => {
                let category = if event_type.contains("Security")
                    || event_type.contains("Violation")
                    || event_type.contains("Capability")
                    || event_type.contains("Approval")
                {
                    EventCategory::Security
                } else if event_type.contains("Mcp") || event_type.contains("Tool") {
                    EventCategory::Tool
                } else if event_type.contains("Llm") || event_type.contains("Request") {
                    EventCategory::Llm
                } else {
                    EventCategory::Runtime
                };

                let event = EventRecord {
                    timestamp: Instant::now(),
                    agent_name: agent.clone(),
                    event_type: event_type.clone(),
                    detail: detail.clone(),
                    category,
                };
                app.nexus_stream.push(NexusEntry::Event(event.clone()));
                app.event_stream.push_back(event);

                // Also add to the regular events
                app.events.push(ActivityEvent {
                    timestamp: Instant::now(),
                    icon: "◐".to_string(),
                    message: format!("{event_type}: {detail}"),
                    category,
                    agent_name: Some(agent.clone()),
                });

                self.next_step();
            },

            DemoStep::SetNexusFilter(category) => {
                app.nexus_filter = match category {
                    NexusCategoryDemo::All => NexusCategory::All,
                    NexusCategoryDemo::Conversation => NexusCategory::Conversation,
                    NexusCategoryDemo::Mcp => NexusCategory::Mcp,
                    NexusCategoryDemo::Security => NexusCategory::Security,
                    NexusCategoryDemo::Audit => NexusCategory::Audit,
                    NexusCategoryDemo::Llm => NexusCategory::Llm,
                    NexusCategoryDemo::Runtime => NexusCategory::Runtime,
                    NexusCategoryDemo::Error => NexusCategory::Error,
                };
                self.next_step();
            },
        }

        false
    }

    fn next_step(&mut self) {
        self.current_step = self.current_step.saturating_add(1);
        self.step_start = Instant::now();
        self.typing_index = 0;
        self.streaming_index = 0;
        self.waiting = false;
    }

    #[allow(dead_code)]
    pub(crate) fn is_complete(&self) -> bool {
        self.current_step >= self.scenario.steps.len()
    }
}
