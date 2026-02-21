//! Demo scenario definitions.
//!
//! These are fully scripted demos that play automatically - showing the user
//! typing, the agent responding, tool calls happening, and approvals being made.
//! No actual user input required - it's like watching a movie of the experience.

mod approval_flow;
mod error_recovery;
mod file_read;
mod file_write;
mod full_demo;
mod multi_agent_ops;
mod multi_tool;
mod quick;
mod showcase;
mod simple_qa;
use std::time::Duration;

/// A demo scenario that plays automatically
#[derive(Debug, Clone)]
pub(crate) struct DemoScenario {
    pub name: String,
    pub description: String,
    pub steps: Vec<DemoStep>,
}

/// A step in a demo scenario
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum DemoStep {
    /// Pause before next step
    Pause(Duration),

    /// Simulate user typing (appears character by character)
    UserTypes { text: String, typing_speed_ms: u64 },

    /// User presses Enter to submit
    UserSubmits,

    /// Agent starts thinking (shows spinner)
    AgentThinking { duration: Duration },

    /// Agent streams a response (word by word)
    AgentStreams { text: String, word_delay_ms: u64 },

    /// Agent requests tool use (shows approval dialog)
    ToolRequest {
        name: String,
        description: String,
        args: Vec<(String, String)>,
        risk: ToolRisk,
    },

    /// User approves/denies tool (simulated keypress)
    UserApproves { choice: ApprovalChoice },

    /// Tool executes (shows spinner, then result)
    ToolExecutes {
        duration: Duration,
        output: Option<String>,
        success: bool,
    },

    /// Show a system message
    SystemMessage(String),

    /// Clear the screen
    Clear,

    // ─── New UI Features ───────────────────────────────────────────
    /// Boot sequence with twinkling stars
    BootSequence {
        /// Show full cinematic boot (true) or compact boot (false)
        cinematic: bool,
        /// Simulated boot checks and their success status
        checks: Vec<(String, bool)>,
    },

    /// Switch to a different view
    SwitchView(View),

    /// Toggle sidebar state
    ToggleSidebar(SidebarState),

    /// Show a status message in the orbit bar
    OrbitStatus(String),

    /// Add a task to the Missions view
    AddTask {
        id: String,
        title: String,
        status: TaskStatus,
    },

    /// Move a task to a different status
    MoveTask { id: String, status: TaskStatus },

    /// Show file in Stellar view with change indicator
    ShowFile { path: String, status: FileStatus },

    /// Add event to event stream
    StreamEvent { icon: String, message: String },

    /// Set the Nexus filter category
    SetNexusFilter(NexusCategoryDemo),

    /// Show inline diff
    ShowDiff {
        file: String,
        removed: Vec<String>,
        added: Vec<String>,
    },

    /// Narrative text (appears as subtle system message, for demo guidance)
    Narrate(String),

    // ─── Multi-Agent Features ────────────────────────────────────
    /// Spawn a new agent
    SpawnAgent { name: String, model: String },

    /// Update agent status
    SetAgentStatus {
        agent: String,
        status: AgentStatusDemo,
    },

    /// Spawn a sub-agent under a parent
    SpawnSubAgent { parent_agent: String, task: String },

    /// Complete a sub-agent
    CompleteSubAgent { id: String, success: bool },

    /// Grant a capability token
    GrantCapability {
        agent: String,
        resource: String,
        scope: String,
        ttl_secs: Option<u64>,
    },

    /// Add a security violation
    SecurityViolation { agent: String, detail: String },

    /// Add an audit entry
    AddAuditEntry {
        agent: String,
        action: String,
        outcome: AuditOutcomeDemo,
    },

    /// Set budget for an agent
    SetBudget { agent: String, spent: f64 },

    /// Set health status for a component
    SetHealth {
        component: String,
        status: HealthStatusDemo,
    },

    /// Set overall threat level
    SetThreatLevel(ThreatLevelDemo),

    /// Add a shield approval (pending approval in Shield view)
    AddShieldApproval {
        agent: String,
        tool: String,
        risk: ToolRisk,
    },

    /// Add event to the event stream/ticker
    AddEventRecord {
        agent: String,
        event_type: String,
        detail: String,
    },
}

/// Agent status for demo steps
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum AgentStatusDemo {
    Ready,
    Busy,
    Paused,
    Error,
    Starting,
}

/// Audit outcome for demo steps
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum AuditOutcomeDemo {
    Success,
    Failure,
    Denied,
    Violation,
}

/// Health status for demo steps
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum HealthStatusDemo {
    Ok,
    Degraded,
    Down,
}

/// Threat level for demo steps
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum ThreatLevelDemo {
    Low,
    Elevated,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToolRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum ApprovalChoice {
    AllowOnce,
    AllowAlways,
    AllowSession,
    Deny,
}

/// Available views in the dashboard
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum View {
    Nexus,    // Unified stream / conversation
    Missions, // Task board
    Stellar,  // File explorer
    Command,  // Agent table with bulk ops
    Topology, // Agent hierarchy tree
    Shield,   // Approval queue processor
    Chain,    // Audit trail
    Pulse,    // Health/budget/performance
    Log,      // Minimal mode
}

/// Nexus filter category for demo steps
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum NexusCategoryDemo {
    All,
    Conversation,
    Mcp,
    Security,
    Audit,
    Llm,
    Runtime,
    Error,
}

/// Sidebar display state
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum SidebarState {
    Expanded,
    Collapsed,
    Hidden,
}

/// Task status for Missions view
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum TaskStatus {
    Backlog,
    Active,
    Review,
    Complete,
    Blocked,
}

/// File change status for Stellar view
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum FileStatus {
    Unchanged,
    Modified,
    Added,
    Deleted,
    Editing,
    NeedsAttention,
}

impl DemoScenario {
    /// Load a scenario by name
    pub(crate) fn load(name: &str) -> Option<Self> {
        match name {
            "simple-qa" => Some(simple_qa::build()),
            "file-read" => Some(file_read::build()),
            "file-write" => Some(file_write::build()),
            "multi-tool" => Some(multi_tool::build()),
            "approval-flow" => Some(approval_flow::build()),
            "error" => Some(error_recovery::build()),
            "full-demo" => Some(full_demo::build()),
            "showcase" => Some(showcase::build()),
            "quick" => Some(quick::build()),
            "multi-agent-ops" => Some(multi_agent_ops::build()),
            _ => None,
        }
    }
    /// List available scenarios
    #[allow(dead_code)]
    pub(crate) fn list() -> Vec<(&'static str, &'static str)> {
        vec![
            ("simple-qa", "Simple question and answer, no tools"),
            ("file-read", "Agent reads a file in workspace"),
            ("file-write", "Agent writes a file (needs approval)"),
            ("multi-tool", "Agent uses multiple tools in sequence"),
            ("approval-flow", "Shows all approval options"),
            ("error", "Tool fails, agent recovers"),
            ("full-demo", "Complete end-to-end showcase"),
            (
                "showcase",
                "Ultimate demo: boot sequence, all views, all features",
            ),
            ("quick", "Quick 30-second demo of key features"),
            (
                "multi-agent-ops",
                "Multi-agent: 3 agents, sub-agents, approvals, audit chain",
            ),
        ]
    }
}
