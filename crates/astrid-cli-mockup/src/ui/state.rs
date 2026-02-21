//! UI state machine and app state.

use super::Term;
use crate::demo::{DemoPlayer, DemoScenario};
use crate::mock;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::time::{Duration, Instant};

/// Current view in the dashboard
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum ViewMode {
    // OPERATE
    #[default]
    Nexus, // Unified stream / conversation (1)
    Missions, // Task board (2)
    Stellar,  // File explorer (3)

    // CONTROL
    Command,  // Agent table with bulk ops (4)
    Topology, // Agent hierarchy tree (5)
    Shield,   // Approval queue processor (6)

    // MONITOR
    Chain, // Audit trail (7)
    Pulse, // Health/budget/performance (8)

    // UTILITY
    Log, // Minimal mode / console (0)
}

impl ViewMode {
    /// Get the number key for this view (1-8, 0 for Log)
    pub(crate) fn number_key(self) -> char {
        match self {
            Self::Nexus => '1',
            Self::Missions => '2',
            Self::Stellar => '3',
            Self::Command => '4',
            Self::Topology => '5',
            Self::Shield => '6',
            Self::Chain => '7',
            Self::Pulse => '8',
            Self::Log => '0',
        }
    }

    /// Get the display name for the sidebar
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Nexus => "Nexus",
            Self::Missions => "Missions",
            Self::Stellar => "Atlas",
            Self::Command => "Command",
            Self::Topology => "Topology",
            Self::Shield => "Shield",
            Self::Chain => "Chain",
            Self::Pulse => "Pulse",
            Self::Log => "Console",
        }
    }

    /// All views in sidebar order
    pub(crate) fn all_ordered() -> &'static [Self] {
        &[
            Self::Nexus,
            Self::Missions,
            Self::Stellar,
            Self::Command,
            Self::Topology,
            Self::Shield,
            Self::Chain,
            Self::Pulse,
            Self::Log,
        ]
    }

    /// Next view in Tab order
    pub(crate) fn next(self) -> Self {
        let all = Self::all_ordered();
        let idx = all.iter().position(|v| *v == self).unwrap_or(0);
        // Safety: all.len() > 0 (hardcoded list), modulo by nonzero
        #[allow(clippy::arithmetic_side_effects)]
        let next_idx = (idx + 1) % all.len();
        all[next_idx]
    }

    /// Previous view in Tab order
    pub(crate) fn prev(self) -> Self {
        let all = Self::all_ordered();
        let idx = all.iter().position(|v| *v == self).unwrap_or(0);
        // Safety: all.len() > 0 (hardcoded list), so idx + all.len() - 1 won't underflow,
        // and modulo by nonzero
        #[allow(clippy::arithmetic_side_effects)]
        let prev_idx = (idx + all.len() - 1) % all.len();
        all[prev_idx]
    }

    /// Map from number key character to `ViewMode`
    pub(crate) fn from_number_key(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::Nexus),
            '2' => Some(Self::Missions),
            '3' => Some(Self::Stellar),
            '4' => Some(Self::Command),
            '5' => Some(Self::Topology),
            '6' => Some(Self::Shield),
            '7' => Some(Self::Chain),
            '8' => Some(Self::Pulse),
            '0' => Some(Self::Log),
            _ => None,
        }
    }

    /// Sidebar section for this view
    pub(crate) fn section(self) -> &'static str {
        match self {
            Self::Nexus | Self::Missions | Self::Stellar => "OPERATE",
            Self::Command | Self::Topology | Self::Shield => "CONTROL",
            Self::Chain | Self::Pulse => "MONITOR",
            Self::Log => "",
        }
    }
}

/// Sidebar display state
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum SidebarMode {
    #[default]
    Expanded, // Full sidebar with labels
    Collapsed, // Icons only
    Hidden,    // No sidebar (Log mode)
}

/// Task column in kanban board
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum TaskColumn {
    #[default]
    Backlog,
    Active,
    Review,
    Complete,
    Blocked,
    Queued, // New: for deferred/queued tasks
}

/// A task in the kanban board
#[derive(Debug, Clone)]
pub(crate) struct Task {
    pub id: String,
    pub title: String,
    pub column: TaskColumn,
    pub agent_name: Option<String>, // Which agent owns this task
}

/// File entry for the Stellar (file explorer) view
#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub path: String,
    pub status: FileEntryStatus,
    pub depth: usize,
    pub is_dir: bool,
}

/// Status of a file in the Stellar view
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FileEntryStatus {
    Unchanged,
    Modified,
    Added,
    Deleted,
    Editing,
}

/// Activity event for the Stream view
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ActivityEvent {
    pub timestamp: Instant,
    pub icon: String,
    pub message: String,
    pub category: EventCategory,
    pub agent_name: Option<String>,
}

/// Category for stream events (determines color)
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum EventCategory {
    Session,
    Tool,
    Approval,
    Error,
    Security,
    Llm,
    Runtime,
}

// ─── Multi-Agent Types ───────────────────────────────────────────

/// Agent status mirroring `AgentStatus` from gateway
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum AgentStatus {
    #[default]
    Ready,
    Busy,
    Paused,
    Error,
    Starting,
}

/// Snapshot of an agent's current state for the UI
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct AgentSnapshot {
    pub name: String,
    pub status: AgentStatus,
    pub last_activity: Instant,
    pub current_activity: Option<String>,
    pub current_tool: Option<String>,
    pub request_count: u64,
    pub last_error: Option<String>,
    pub context_usage: f32,
    pub budget_spent: f64,
    pub active_subagents: usize,
    pub pending_approvals: usize,
    pub tokens_used: usize,
}

/// Sub-agent status mirroring `SubAgentStatus`
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub(crate) enum SubAgentStatus {
    Running,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

/// A node in the sub-agent hierarchy tree
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct SubAgentNode {
    pub id: String,
    pub parent_agent: String,
    pub parent_subagent: Option<String>,
    pub task: String,
    pub status: SubAgentStatus,
    pub depth: usize,
    pub started_at: Instant,
    pub duration: Option<Duration>,
    pub expanded: bool,
}

/// Snapshot of a capability token for Shield view
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct CapabilitySnapshot {
    pub id: String,
    pub resource: String,
    pub permissions: Vec<String>,
    pub scope: String,
    pub expires_in: Option<Duration>,
    pub use_count: usize,
    pub agent_name: String,
}

/// Approval snapshot for Shield view (different from the approval overlay)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ApprovalSnapshot {
    pub id: usize,
    pub agent_name: String,
    pub tool_name: String,
    pub risk_level: RiskLevel,
    pub description: String,
    pub timestamp: Instant,
}

/// Record of a denied action
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct DenialRecord {
    pub agent_name: String,
    pub tool_name: String,
    pub risk_level: RiskLevel,
    pub timestamp: Instant,
}

/// Threat level for the security dashboard
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum ThreatLevel {
    #[default]
    Low,
    Elevated,
    High,
    Critical,
}

impl ThreatLevel {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Low => "LOW",
            Self::Elevated => "ELEVATED",
            Self::High => "HIGH",
            Self::Critical => "CRITICAL",
        }
    }
}

/// Budget tracking state
#[derive(Debug, Clone, Default)]
pub(crate) struct BudgetState {
    pub session_limit: f64,
    pub total_spent: f64,
    pub per_agent: HashMap<String, f64>,
    pub burn_rate_per_hour: f64,
    pub input_tokens: usize,
    pub output_tokens: usize,
}

/// Performance metrics
#[derive(Debug, Clone, Default)]
pub(crate) struct PerformanceMetrics {
    pub avg_tool_latency_ms: f64,
    pub avg_llm_latency_ms: f64,
    pub avg_approval_wait_ms: f64,
    pub tool_calls_per_min: f64,
    pub events_per_min: f64,
}

/// Health check status for a component
#[derive(Debug, Clone)]
pub(crate) struct HealthCheck {
    pub component: String,
    pub status: HealthStatus,
    pub latency_ms: f64,
}

/// Health status of a component
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HealthStatus {
    Ok,
    Degraded,
    Down,
}

/// Overall health snapshot
#[derive(Debug, Clone, Default)]
pub(crate) struct HealthSnapshot {
    pub checks: Vec<HealthCheck>,
    pub overall: OverallHealth,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum OverallHealth {
    #[default]
    Healthy,
    Degraded,
    Unhealthy,
}

/// Audit entry snapshot for Chain view
#[derive(Debug, Clone)]
pub(crate) struct AuditSnapshot {
    pub id: usize,
    pub timestamp: Instant,
    pub agent_name: String,
    pub action: String,
    pub auth_method: String,
    pub outcome: AuditOutcome,
    pub detail: String,
    pub hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AuditOutcome {
    Success,
    Failure,
    Denied,
    Violation,
}

/// Chain integrity status
#[derive(Debug, Clone, Default)]
pub(crate) struct ChainIntegrity {
    pub verified: bool,
    pub total_entries: usize,
    pub break_at: Option<usize>,
}

/// Filter for the audit chain view
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum AuditFilter {
    #[default]
    All,
    Security,
    Tools,
    Sessions,
    Llm,
}

impl AuditFilter {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Security => "Security",
            Self::Tools => "Tools",
            Self::Sessions => "Sessions",
            Self::Llm => "LLM",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::All => Self::Security,
            Self::Security => Self::Tools,
            Self::Tools => Self::Sessions,
            Self::Sessions => Self::Llm,
            Self::Llm => Self::All,
        }
    }
}

/// Filter for the Stream/event view
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(dead_code)]
pub(crate) enum EventFilter {
    #[default]
    All,
    Runtime,
    Mcp,
    Security,
    Llm,
    Error,
}

#[allow(dead_code)]
impl EventFilter {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Runtime => "Runtime",
            Self::Mcp => "MCP",
            Self::Security => "Security",
            Self::Llm => "LLM",
            Self::Error => "Error",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::All => Self::Runtime,
            Self::Runtime => Self::Mcp,
            Self::Mcp => Self::Security,
            Self::Security => Self::Llm,
            Self::Llm => Self::Error,
            Self::Error => Self::All,
        }
    }
}

// ─── Nexus Unified Stream Types ─────────────────────────────────

/// A single entry in the Nexus unified timeline
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum NexusEntry {
    Message(Message),
    Event(EventRecord),
    Approval(ApprovalSnapshot),
    SubAgentLifecycle {
        timestamp: Instant,
        agent: String,
        subagent_id: String,
        action: String,
        status: SubAgentStatus,
    },
    AuditEntry(AuditSnapshot),
    AgentSpawned {
        timestamp: Instant,
        name: String,
        model: String,
    },
    SecurityAlert {
        timestamp: Instant,
        agent: String,
        detail: String,
        level: ThreatLevel,
    },
    SystemNotice {
        timestamp: Instant,
        content: String,
    },
}

impl NexusEntry {
    #[allow(dead_code)]
    pub(crate) fn timestamp(&self) -> Instant {
        match self {
            Self::Message(m) => m.timestamp,
            Self::Event(e) => e.timestamp,
            Self::Approval(a) => a.timestamp,
            Self::AuditEntry(a) => a.timestamp,
            Self::SubAgentLifecycle { timestamp, .. }
            | Self::AgentSpawned { timestamp, .. }
            | Self::SecurityAlert { timestamp, .. }
            | Self::SystemNotice { timestamp, .. } => *timestamp,
        }
    }

    pub(crate) fn category(&self) -> NexusCategory {
        match self {
            Self::Message(_) => NexusCategory::Conversation,
            Self::Event(e) => match e.category {
                EventCategory::Tool => NexusCategory::Mcp,
                EventCategory::Security | EventCategory::Approval => NexusCategory::Security,
                EventCategory::Llm => NexusCategory::Llm,
                EventCategory::Error => NexusCategory::Error,
                EventCategory::Runtime | EventCategory::Session => NexusCategory::Runtime,
            },
            Self::Approval(_) | Self::SecurityAlert { .. } => NexusCategory::Security,
            Self::SubAgentLifecycle { .. }
            | Self::AgentSpawned { .. }
            | Self::SystemNotice { .. } => NexusCategory::Runtime,
            Self::AuditEntry(_) => NexusCategory::Audit,
        }
    }

    pub(crate) fn agent_name(&self) -> Option<&str> {
        match self {
            Self::Message(_) | Self::SystemNotice { .. } => None,
            Self::Event(e) => Some(&e.agent_name),
            Self::Approval(a) => Some(&a.agent_name),
            Self::SubAgentLifecycle { agent, .. } | Self::SecurityAlert { agent, .. } => {
                Some(agent)
            },
            Self::AuditEntry(a) => Some(&a.agent_name),
            Self::AgentSpawned { name, .. } => Some(name),
        }
    }
}

/// Filter category for the Nexus unified stream
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum NexusCategory {
    #[default]
    All,
    Conversation,
    Mcp,
    Security,
    Audit,
    Llm,
    Runtime,
    Error,
}

impl NexusCategory {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Conversation => "Chat",
            Self::Mcp => "MCP",
            Self::Security => "Security",
            Self::Audit => "Audit",
            Self::Llm => "LLM",
            Self::Runtime => "Runtime",
            Self::Error => "Error",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::All => Self::Conversation,
            Self::Conversation => Self::Mcp,
            Self::Mcp => Self::Security,
            Self::Security => Self::Audit,
            Self::Audit => Self::Llm,
            Self::Llm => Self::Runtime,
            Self::Runtime => Self::Error,
            Self::Error => Self::All,
        }
    }
}

// ─── Command Table Types ────────────────────────────────────────

/// Sort column for the Command table view
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(dead_code)]
pub(crate) enum CommandSort {
    #[default]
    Name,
    Status,
    Activity,
    Budget,
    SubAgents,
    Context,
}

impl CommandSort {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Status => "Status",
            Self::Activity => "Activity",
            Self::Budget => "Budget",
            Self::SubAgents => "Sub",
            Self::Context => "Ctx%",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::Name => Self::Status,
            Self::Status => Self::Activity,
            Self::Activity => Self::Budget,
            Self::Budget => Self::SubAgents,
            Self::SubAgents => Self::Context,
            Self::Context => Self::Name,
        }
    }
}

/// Sort direction for tables
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

impl SortDirection {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    pub(crate) fn arrow(self) -> &'static str {
        match self {
            Self::Ascending => "↑",
            Self::Descending => "↓",
        }
    }
}

// ─── Shield Queue Types ─────────────────────────────────────────

/// Sort for Shield approval queue
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(dead_code)]
pub(crate) enum ShieldSort {
    #[default]
    Risk,
    Agent,
    Time,
}

/// Per-agent conversation state (for Nexus agent switching)
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub(crate) struct ConversationState {
    pub messages: Vec<Message>,
    pub scroll_offset: usize,
}

/// Main application state
#[allow(clippy::struct_excessive_bools, dead_code)]
pub(crate) struct App {
    /// Current UI state
    pub state: UiState,
    /// Current view mode
    pub view: ViewMode,
    /// Sidebar state
    pub sidebar: SidebarMode,
    /// Conversation messages
    pub messages: Vec<Message>,
    /// Current input buffer
    pub input: String,
    /// Input cursor position
    pub cursor_pos: usize,
    /// Whether app should quit
    pub should_quit: bool,
    /// Pending tool calls awaiting approval
    pub pending_approvals: Vec<ApprovalRequest>,
    /// Currently running tools
    pub running_tools: Vec<ToolStatus>,
    /// Completed tool calls
    pub completed_tools: Vec<ToolStatus>,
    /// Demo player (if running in demo mode)
    pub demo_player: Option<DemoPlayer>,
    /// Last render time (for debouncing)
    pub last_render: Instant,
    /// Streaming buffer
    pub stream_buffer: String,
    /// Currently selected approval (for keyboard navigation)
    pub selected_approval: usize,
    /// Scroll offset from bottom (0 = at bottom, showing most recent)
    pub scroll_offset: usize,
    /// Current working directory
    pub working_dir: String,
    /// Current model name
    pub model_name: String,
    /// Context usage (0.0 - 1.0)
    pub context_usage: f32,
    /// Tokens streamed in current response
    pub tokens_streamed: usize,
    /// Whether sandbox is enabled
    pub sandbox_enabled: bool,
    /// Tasks for kanban board (Missions view)
    pub tasks: Vec<Task>,
    /// Files for Stellar view
    pub files: Vec<FileEntry>,
    /// Activity events for Stream view
    pub events: Vec<ActivityEvent>,
    /// Git branch name (for status bar)
    #[allow(dead_code)]
    pub git_branch: String,
    /// Last completed activity (past-tense verb, duration) for greyed-out display
    pub last_completed: Option<(String, Duration)>,
    /// When the last completed activity finished (for fade-out timing)
    pub last_completed_at: Option<Instant>,
    /// Whether the welcome screen is currently showing
    pub welcome_visible: bool,
    /// Username for welcome greeting
    pub username: String,
    /// Whether Ctrl+C was pressed once (waiting for confirmation)
    pub quit_pending: bool,

    // ─── Multi-Agent State ───────────────────────────────────────
    /// Agent snapshots for Command/Topology views
    pub agents: Vec<AgentSnapshot>,
    /// Currently selected agent in Command grid
    pub selected_agent: usize,
    /// Focused agent for detail view (Enter on grid)
    pub focused_agent: Option<usize>,
    /// Per-agent conversation histories for Nexus switching
    #[allow(dead_code)]
    pub conversations: HashMap<String, ConversationState>,

    /// Sub-agent tree for Topology view
    pub subagent_tree: Vec<SubAgentNode>,

    // ─── Nexus Unified Stream ────────────────────────────────────
    /// Unified timeline of all events
    pub nexus_stream: Vec<NexusEntry>,
    /// Current filter category
    pub nexus_filter: NexusCategory,
    /// Filter to specific agent (None = all)
    pub nexus_agent_filter: Option<String>,
    /// Text search within Nexus
    pub nexus_search: Option<String>,
    /// Target agent for conversation (who we're talking to)
    pub nexus_target_agent: Option<String>,

    // ─── Command Table ───────────────────────────────────────────
    /// Sort column for Command table
    pub command_sort: CommandSort,
    /// Sort direction
    pub command_sort_dir: SortDirection,
    /// Filter text for command view
    pub command_filter: Option<String>,
    /// Multi-selected agent indices
    pub command_selected: Vec<usize>,

    // ─── Shield Queue ────────────────────────────────────────────
    /// Security approvals for Shield view
    pub shield_approvals: Vec<ApprovalSnapshot>,
    /// Active capability tokens for Shield view
    pub active_capabilities: Vec<CapabilitySnapshot>,
    /// Recent denials for Shield view
    pub recent_denials: Vec<DenialRecord>,
    /// Current threat level
    pub threat_level: ThreatLevel,
    /// Selected item in Shield queue
    pub shield_selected: usize,
    /// Shield sort order
    pub shield_sort: ShieldSort,
    /// Shield risk filter (None = all)
    pub shield_risk_filter: Option<RiskLevel>,
    /// Multi-selected items in Shield
    pub shield_selected_items: Vec<usize>,
    /// Whether detail panel is expanded
    pub shield_detail_expanded: bool,

    /// Audit entries for Chain view (ring buffer)
    pub audit_entries: VecDeque<AuditSnapshot>,
    /// Chain integrity status
    pub chain_integrity: ChainIntegrity,
    /// Current audit filter
    pub audit_filter: AuditFilter,
    /// Audit agent filter (None = all agents)
    pub audit_agent_filter: Option<String>,
    /// Scroll offset for audit chain
    pub audit_scroll: usize,

    /// Event stream (ring buffer) for Command ticker
    pub event_stream: VecDeque<EventRecord>,
    /// Event filter for the Command view event ticker
    pub event_filter: EventFilter,
    /// Event agent filter (None = all agents)
    pub event_agent_filter: Option<String>,

    /// Health snapshot for Pulse view
    pub health: HealthSnapshot,
    /// Budget tracking for Pulse view
    pub budget: BudgetState,
    /// Performance metrics for Pulse view
    pub performance: PerformanceMetrics,
    /// Gateway uptime
    pub gateway_uptime: Duration,
}

/// An event record for the ticker / stream
#[derive(Debug, Clone)]
pub(crate) struct EventRecord {
    pub timestamp: Instant,
    pub agent_name: String,
    pub event_type: String,
    pub detail: String,
    pub category: EventCategory,
}

/// UI state machine states
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) enum UiState {
    /// Waiting for user input
    Idle,
    /// Agent is thinking/processing
    Thinking { start_time: Instant, dots: usize },
    /// Awaiting approval for tool use
    AwaitingApproval,
    /// Tool is currently running
    ToolRunning {
        tool_name: String,
        start_time: Instant,
    },
    /// Streaming response from agent
    Streaming { start_time: Instant },
    /// Error state
    Error { message: String },
    /// Interrupted by user (Esc during thinking/streaming)
    Interrupted,
}

/// A conversation message
#[derive(Debug, Clone)]
pub(crate) struct Message {
    pub role: MessageRole,
    pub content: String,
    #[allow(dead_code)]
    pub timestamp: Instant,
    /// Optional kind for specialized rendering (e.g., diff lines)
    pub kind: Option<MessageKind>,
    /// Whether to add blank line after this message (default: true)
    pub spacing: bool,
}

/// Special message kinds for styled rendering
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum MessageKind {
    /// Diff header line
    DiffHeader,
    /// Diff removed line
    DiffRemoved,
    /// Diff added line
    DiffAdded,
    /// Diff footer line
    DiffFooter,
    /// Inline tool result (index into `completed_tools`)
    ToolResult(usize),
}

/// Message sender role
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MessageRole {
    User,
    Assistant,
    System,
}

/// A pending approval request
#[derive(Debug, Clone)]
pub(crate) struct ApprovalRequest {
    pub id: usize,
    pub tool_name: String,
    pub description: String,
    pub risk_level: RiskLevel,
    pub details: Vec<(String, String)>,
}

/// Risk level for tool calls
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RiskLevel {
    Low,    // Read-only, in workspace
    Medium, // Write, but in workspace
    High,   // Outside workspace or sensitive
}

/// Status of a tool execution
#[derive(Debug, Clone)]
pub(crate) struct ToolStatus {
    pub name: String,
    /// Primary argument for display, e.g. "src/auth.rs" for `read_file`
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

impl App {
    /// Create a new app instance
    pub(crate) fn new() -> Self {
        // Get current working directory, fallback to "."
        let working_dir =
            std::env::current_dir().map_or_else(|_| ".".to_string(), |p| p.display().to_string());

        Self {
            state: UiState::Idle,
            view: ViewMode::default(),
            sidebar: SidebarMode::default(),
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            should_quit: false,
            pending_approvals: Vec::new(),
            running_tools: Vec::new(),
            completed_tools: Vec::new(),
            demo_player: None,
            last_render: Instant::now(),
            stream_buffer: String::new(),
            selected_approval: 0,
            scroll_offset: 0,
            working_dir,
            model_name: "claude-opus-4.6".to_string(),
            context_usage: 0.0,
            tokens_streamed: 0,
            sandbox_enabled: false, // Demo: no sandbox
            tasks: Vec::new(),
            files: Vec::new(),
            events: Vec::new(),
            git_branch: "main".to_string(),
            last_completed: None,
            last_completed_at: None,
            welcome_visible: false,
            username: std::env::var("USER").unwrap_or_else(|_| "Pilot".to_string()),
            quit_pending: false,

            // Multi-agent state
            agents: Vec::new(),
            selected_agent: 0,
            focused_agent: None,
            conversations: HashMap::new(),

            subagent_tree: Vec::new(),

            // Nexus unified stream
            nexus_stream: Vec::new(),
            nexus_filter: NexusCategory::default(),
            nexus_agent_filter: None,
            nexus_search: None,
            nexus_target_agent: None,

            // Command table
            command_sort: CommandSort::default(),
            command_sort_dir: SortDirection::default(),
            command_filter: None,
            command_selected: Vec::new(),

            // Shield queue
            shield_approvals: Vec::new(),
            active_capabilities: Vec::new(),
            recent_denials: Vec::new(),
            threat_level: ThreatLevel::default(),
            shield_selected: 0,
            shield_sort: ShieldSort::default(),
            shield_risk_filter: None,
            shield_selected_items: Vec::new(),
            shield_detail_expanded: false,

            audit_entries: VecDeque::new(),
            chain_integrity: ChainIntegrity::default(),
            audit_filter: AuditFilter::default(),
            audit_agent_filter: None,
            audit_scroll: 0,

            event_stream: VecDeque::new(),
            event_filter: EventFilter::default(),
            event_agent_filter: None,

            health: HealthSnapshot::default(),
            budget: BudgetState {
                session_limit: 30.0,
                ..Default::default()
            },
            performance: PerformanceMetrics::default(),
            gateway_uptime: Duration::ZERO,
        }
    }

    /// Load a demo scenario
    pub(crate) fn load_demo(&mut self, scenario_name: &str) {
        if let Some(scenario) = DemoScenario::load(scenario_name) {
            self.messages.push(Message {
                role: MessageRole::System,
                content: format!("Demo mode: {} - {}", scenario.name, scenario.description),
                timestamp: Instant::now(),
                kind: None,
                spacing: true,
            });
            self.demo_player = Some(DemoPlayer::new(scenario));
        } else {
            self.messages.push(Message {
                role: MessageRole::System,
                content: format!("Unknown scenario: {scenario_name}. Available: simple-qa, file-read, file-write, multi-tool, approval-flow, error, full-demo, showcase, quick, multi-agent-ops."),
                timestamp: Instant::now(),
                kind: None,
                spacing: true,
            });
        }
    }

    /// Main run loop
    pub(crate) fn run(&mut self, terminal: &mut Term) -> io::Result<()> {
        // Render interval (60fps max, but we debounce)
        let render_interval = Duration::from_millis(16);

        loop {
            // Render if enough time has passed
            if self.last_render.elapsed() >= render_interval {
                terminal.draw(|frame| super::render_frame(frame, self))?;
                self.last_render = Instant::now();
            }

            // Handle input with a small timeout to allow responsive rendering
            if crossterm::event::poll(Duration::from_millis(10))? {
                super::handle_input(self)?;
            }

            // Update state (animations, timeouts, etc.)
            self.update_state();

            // Check if we should quit
            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    /// Update state for animations and async operations
    fn update_state(&mut self) {
        // Advance demo player if running
        if self.demo_player.is_some() {
            // Take ownership to avoid borrow conflict
            let mut player = self.demo_player.take().expect("mockup error");
            let complete = player.advance(self);
            if complete {
                // Demo complete
                self.messages.push(Message {
                    role: MessageRole::System,
                    content: "Demo complete. Press Ctrl+C to exit or /clear to start fresh."
                        .to_string(),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
            } else {
                // Put player back
                self.demo_player = Some(player);
            }
            return; // Demo player manages all state updates
        }

        // Update thinking animation (non-demo mode)
        if let UiState::Thinking { start_time, dots } = &self.state {
            let elapsed = start_time.elapsed();
            let new_dots = ((elapsed.as_millis() / 500) % 4) as usize;
            if new_dots != *dots {
                self.state = UiState::Thinking {
                    start_time: *start_time,
                    dots: new_dots,
                };
            }
        }
    }

    /// Submit user input
    pub(crate) fn submit_input(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }

        let content = std::mem::take(&mut self.input);
        self.cursor_pos = 0;

        // Handle commands
        if content.starts_with('/') {
            self.handle_command(&content);
            return;
        }

        // Add user message
        let msg = Message {
            role: MessageRole::User,
            content: content.clone(),
            timestamp: Instant::now(),
            kind: None,
            spacing: true,
        };
        self.nexus_stream.push(NexusEntry::Message(msg.clone()));
        self.messages.push(msg);

        // Start thinking
        self.state = UiState::Thinking {
            start_time: Instant::now(),
            dots: 0,
        };

        // In real impl, this would trigger LLM call
        // In mockup, we wait for demo or timeout
    }

    /// Handle slash commands
    fn handle_command(&mut self, cmd: &str) {
        match cmd.trim() {
            "/quit" | "/exit" | "/q" => self.should_quit = true,
            "/clear" => {
                self.messages.clear();
                self.completed_tools.clear();
                self.nexus_stream.clear();
            },
            "/help" => {
                self.messages.push(Message {
                    role: MessageRole::System,
                    content: "Commands: /quit, /clear, /help, /demo <scenario>".to_string(),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
            },
            cmd if cmd.starts_with("/demo ") => {
                let scenario = cmd.strip_prefix("/demo ").unwrap_or("");
                self.load_demo(scenario);
            },
            _ => {
                self.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("Unknown command: {cmd}"),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
            },
        }
    }

    /// Complete thinking and move to next state (interactive mode)
    #[allow(dead_code)]
    fn complete_thinking(&mut self) {
        // Generate mock response
        let response = mock::generate_response(&self.messages);

        // Check if response includes tool calls
        if let Some(tool_call) = mock::extract_tool_call(&response) {
            self.pending_approvals.push(ApprovalRequest {
                id: self.pending_approvals.len(),
                tool_name: tool_call.name,
                description: tool_call.description,
                risk_level: tool_call.risk,
                details: tool_call.details,
            });
            self.state = UiState::AwaitingApproval;
        } else {
            // Start streaming the response
            self.stream_buffer = response;
            self.state = UiState::Streaming {
                start_time: Instant::now(),
            };
        }
    }

    /// Stream next chunk of response (interactive mode)
    #[allow(dead_code)]
    fn stream_next_chunk(&mut self) {
        if self.stream_buffer.is_empty() {
            // Done streaming
            self.state = UiState::Idle;
            return;
        }

        // Take a chunk (word-based for natural feel)
        let chunk_size = self
            .stream_buffer
            .find(' ')
            .unwrap_or(self.stream_buffer.len());
        let chunk_size = chunk_size.saturating_add(1).min(self.stream_buffer.len());
        let chunk: String = self.stream_buffer.drain(..chunk_size).collect();

        // Add to last assistant message or create new one
        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::Assistant {
                last.content.push_str(&chunk);
            } else {
                self.messages.push(Message {
                    role: MessageRole::Assistant,
                    content: chunk,
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
            }
        } else {
            self.messages.push(Message {
                role: MessageRole::Assistant,
                content: chunk,
                timestamp: Instant::now(),
                kind: None,
                spacing: true,
            });
        }

        // Update streaming state
        self.state = UiState::Streaming {
            start_time: Instant::now(),
        };
    }

    /// Approve a pending tool call
    pub(crate) fn approve_tool(&mut self, id: usize, always: bool) {
        if let Some(pos) = self.pending_approvals.iter().position(|a| a.id == id) {
            let approval = self.pending_approvals.remove(pos);

            // Extract primary arg for display
            let display_arg = approval
                .details
                .first()
                .map(|(_, v)| v.clone())
                .unwrap_or_default();

            // Add to running tools
            self.running_tools.push(ToolStatus {
                name: approval.tool_name.clone(),
                display_arg,
                status: ToolStatusKind::Running,
                start_time: Instant::now(),
                end_time: None,
                output: None,
                expanded: false,
            });

            // Update state
            self.state = UiState::ToolRunning {
                tool_name: approval.tool_name,
                start_time: Instant::now(),
            };

            if always {
                self.messages.push(Message {
                    role: MessageRole::System,
                    content: "Capability token created for future use.".to_string(),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
            }
        }

        // Check if more approvals pending
        if self.pending_approvals.is_empty() && self.running_tools.is_empty() {
            self.state = UiState::Idle;
        } else if !self.pending_approvals.is_empty() {
            self.state = UiState::AwaitingApproval;
        }
    }

    /// Deny a pending tool call
    pub(crate) fn deny_tool(&mut self, id: usize) {
        self.pending_approvals.retain(|a| a.id != id);

        // Add denial message
        self.messages.push(Message {
            role: MessageRole::System,
            content: "Tool call denied.".to_string(),
            timestamp: Instant::now(),
            kind: None,
            spacing: true,
        });

        // Update state
        if self.pending_approvals.is_empty() {
            self.state = UiState::Idle;
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
