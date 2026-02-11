//! Demo scenario definitions.
//!
//! These are fully scripted demos that play automatically - showing the user
//! typing, the agent responding, tool calls happening, and approvals being made.
//! No actual user input required - it's like watching a movie of the experience.

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
            "simple-qa" => Some(Self::simple_qa()),
            "file-read" => Some(Self::file_read()),
            "file-write" => Some(Self::file_write()),
            "multi-tool" => Some(Self::multi_tool()),
            "approval-flow" => Some(Self::approval_flow()),
            "error" => Some(Self::error_recovery()),
            "full-demo" => Some(Self::full_demo()),
            "showcase" => Some(Self::showcase()),
            "quick" => Some(Self::quick()),
            "multi-agent-ops" => Some(Self::multi_agent_ops()),
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

    /// Simple Q&A - no tools involved
    fn simple_qa() -> Self {
        Self {
            name: "simple-qa".to_string(),
            description: "Simple question and answer without tool use".to_string(),
            steps: vec![
                DemoStep::SystemMessage("Demo: Simple Q&A".to_string()),
                DemoStep::Pause(Duration::from_secs(1)),
                // User types a question
                DemoStep::UserTypes {
                    text: "What is a state machine?".to_string(),
                    typing_speed_ms: 50,
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::UserSubmits,
                // Agent thinks
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(1500),
                },
                // Agent responds
                DemoStep::AgentStreams {
                    text: "A state machine is a computational model that can be in exactly one of a finite number of states at any given time. It transitions between states based on inputs or events.\n\nKey components:\n\n1. **States** - The possible conditions the system can be in\n2. **Transitions** - Rules for moving between states\n3. **Events** - Triggers that cause transitions\n\nThey're useful for modeling UI flows, parsers, and game logic.".to_string(),
                    word_delay_ms: 30,
                },
                DemoStep::Pause(Duration::from_secs(2)),
            ],
        }
    }

    /// Agent reads a file in workspace (auto-approved or low-risk)
    fn file_read() -> Self {
        Self {
            name: "file-read".to_string(),
            description: "Agent reads a file from the workspace".to_string(),
            steps: vec![
                DemoStep::SystemMessage("Demo: Reading a file".to_string()),
                DemoStep::Pause(Duration::from_secs(1)),
                // User asks about code
                DemoStep::UserTypes {
                    text: "Can you explain the main function?".to_string(),
                    typing_speed_ms: 45,
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::UserSubmits,
                // Agent thinks briefly
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(800),
                },
                // Agent wants to read file
                DemoStep::AgentStreams {
                    text: "I'll read the main file to explain it.".to_string(),
                    word_delay_ms: 40,
                },
                DemoStep::Pause(Duration::from_millis(500)),
                // Tool request
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "src/main.rs".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(800)),
                // User approves
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowOnce,
                },
                // Tool runs
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(300),
                    output: Some("fn main() {\n    let app = App::new();\n    app.run();\n}".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(500)),
                // Agent explains
                DemoStep::AgentStreams {
                    text: "The main function is straightforward:\n\n1. Creates a new `App` instance\n2. Calls `run()` to start the application\n\nThis is the entry point that bootstraps the entire program.".to_string(),
                    word_delay_ms: 35,
                },
                DemoStep::Pause(Duration::from_secs(2)),
            ],
        }
    }

    /// Agent writes a file (needs explicit approval)
    fn file_write() -> Self {
        Self {
            name: "file-write".to_string(),
            description: "Agent writes a file (requires approval)".to_string(),
            steps: vec![
                DemoStep::SystemMessage("Demo: Writing a file".to_string()),
                DemoStep::Pause(Duration::from_secs(1)),
                // User asks to fix something
                DemoStep::UserTypes {
                    text: "Add error handling to the connection function".to_string(),
                    typing_speed_ms: 40,
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::UserSubmits,
                // Agent thinks
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(1200),
                },
                // Agent explains plan
                DemoStep::AgentStreams {
                    text: "I'll add proper error handling with Result types.".to_string(),
                    word_delay_ms: 40,
                },
                DemoStep::Pause(Duration::from_millis(500)),
                // Tool request - write file (medium risk)
                DemoStep::ToolRequest {
                    name: "write_file".to_string(),
                    description: "Modify file contents".to_string(),
                    args: vec![
                        ("path".to_string(), "src/connection.rs".to_string()),
                        ("changes".to_string(), "+15 lines, -3 lines".to_string()),
                    ],
                    risk: ToolRisk::Medium,
                },
                DemoStep::Pause(Duration::from_millis(1500)),
                // User approves
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowOnce,
                },
                // Tool executes
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(400),
                    output: None,
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Agent confirms
                DemoStep::AgentStreams {
                    text: "Done. I've wrapped the connection logic in a Result and added proper error propagation. The function now returns `Result<Connection, ConnectionError>` instead of panicking.".to_string(),
                    word_delay_ms: 30,
                },
                DemoStep::Pause(Duration::from_secs(2)),
            ],
        }
    }

    /// Multiple tools in sequence
    fn multi_tool() -> Self {
        Self {
            name: "multi-tool".to_string(),
            description: "Agent uses multiple tools to complete a task".to_string(),
            steps: vec![
                DemoStep::SystemMessage("Demo: Multi-step task".to_string()),
                DemoStep::Pause(Duration::from_secs(1)),
                // User asks for refactor
                DemoStep::UserTypes {
                    text: "Refactor the auth module to use the new User type".to_string(),
                    typing_speed_ms: 42,
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::UserSubmits,
                // Agent thinks
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(1000),
                },
                DemoStep::AgentStreams {
                    text: "I'll need to check the current implementation first.".to_string(),
                    word_delay_ms: 40,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // First tool - read
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "src/auth.rs".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(600)),
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowSession,
                },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(250),
                    output: Some("// Current auth implementation...".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Agent continues
                DemoStep::AgentStreams {
                    text: "I see the structure. Let me also check the User type.".to_string(),
                    word_delay_ms: 35,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Second tool - read another file
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "src/types/user.rs".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Auto-approved because of session permission
                DemoStep::SystemMessage("Auto-approved (session permission)".to_string()),
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(200),
                    output: Some("pub struct User { ... }".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Agent explains and writes
                DemoStep::AgentStreams {
                    text: "Now I'll update auth.rs to use the new User type.".to_string(),
                    word_delay_ms: 35,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Third tool - write
                DemoStep::ToolRequest {
                    name: "write_file".to_string(),
                    description: "Modify file contents".to_string(),
                    args: vec![
                        ("path".to_string(), "src/auth.rs".to_string()),
                        ("changes".to_string(), "+8 lines, -12 lines".to_string()),
                    ],
                    risk: ToolRisk::Medium,
                },
                DemoStep::Pause(Duration::from_millis(1000)),
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowOnce,
                },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(350),
                    output: None,
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::AgentStreams {
                    text: "Refactoring complete. The auth module now uses `User` instead of the old `UserData` struct.".to_string(),
                    word_delay_ms: 30,
                },
                DemoStep::Pause(Duration::from_secs(2)),
            ],
        }
    }

    /// Shows all approval options
    fn approval_flow() -> Self {
        Self {
            name: "approval-flow".to_string(),
            description: "Demonstrates all approval options".to_string(),
            steps: vec![
                DemoStep::SystemMessage("Demo: Approval options".to_string()),
                DemoStep::Pause(Duration::from_secs(1)),
                DemoStep::SystemMessage("Options: [y] Allow once  [a] Allow always  [s] Allow session  [n] Deny".to_string()),
                DemoStep::Pause(Duration::from_secs(2)),
                // First request - deny
                DemoStep::UserTypes {
                    text: "Delete the temp files".to_string(),
                    typing_speed_ms: 50,
                },
                DemoStep::UserSubmits,
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(600),
                },
                DemoStep::ToolRequest {
                    name: "delete_files".to_string(),
                    description: "Delete files matching pattern".to_string(),
                    args: vec![("pattern".to_string(), "*.tmp".to_string())],
                    risk: ToolRisk::High,
                },
                DemoStep::Pause(Duration::from_millis(2000)),
                DemoStep::UserApproves {
                    choice: ApprovalChoice::Deny,
                },
                DemoStep::Pause(Duration::from_millis(500)),
                DemoStep::AgentStreams {
                    text: "Understood. I won't delete those files. Would you like me to list them instead?".to_string(),
                    word_delay_ms: 35,
                },
                DemoStep::Pause(Duration::from_secs(2)),
            ],
        }
    }

    /// Tool fails, agent recovers
    fn error_recovery() -> Self {
        Self {
            name: "error".to_string(),
            description: "Tool fails and agent adapts".to_string(),
            steps: vec![
                DemoStep::SystemMessage("Demo: Error recovery".to_string()),
                DemoStep::Pause(Duration::from_secs(1)),
                DemoStep::UserTypes {
                    text: "Read the config file".to_string(),
                    typing_speed_ms: 50,
                },
                DemoStep::UserSubmits,
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(600),
                },
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "config.yaml".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(600)),
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowOnce,
                },
                // Tool fails
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(300),
                    output: Some("Error: File not found".to_string()),
                    success: false,
                },
                DemoStep::Pause(Duration::from_millis(600)),
                // Agent adapts
                DemoStep::AgentStreams {
                    text: "The config file doesn't exist. Let me check for alternative config locations.".to_string(),
                    word_delay_ms: 35,
                },
                DemoStep::Pause(Duration::from_millis(500)),
                // Try another location
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), ".config/settings.yaml".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(600)),
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowOnce,
                },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(250),
                    output: Some("database:\n  host: localhost\n  port: 5432".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::AgentStreams {
                    text: "Found it at `.config/settings.yaml`. Here's the configuration:\n\n- Database host: localhost\n- Database port: 5432".to_string(),
                    word_delay_ms: 30,
                },
                DemoStep::Pause(Duration::from_secs(2)),
            ],
        }
    }

    /// Complete end-to-end showcase
    fn full_demo() -> Self {
        Self {
            name: "full-demo".to_string(),
            description: "Complete end-to-end showcase of all features".to_string(),
            steps: vec![
                DemoStep::SystemMessage("Astralis Interactive CLI Demo".to_string()),
                DemoStep::Pause(Duration::from_secs(2)),
                DemoStep::Clear,
                // Start with simple greeting
                DemoStep::UserTypes {
                    text: "Hello! Can you help me fix a bug in my auth code?".to_string(),
                    typing_speed_ms: 45,
                },
                DemoStep::UserSubmits,
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(1000),
                },
                DemoStep::AgentStreams {
                    text: "Of course! I'll take a look at your auth code. Let me read it first.".to_string(),
                    word_delay_ms: 35,
                },
                DemoStep::Pause(Duration::from_millis(500)),
                // Read file
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "src/auth.rs".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(800)),
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowSession,
                },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(300),
                    output: Some("pub fn login(user: &str, pass: &str) -> bool {\n    pass == \"admin\"\n}".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(500)),
                DemoStep::AgentStreams {
                    text: "I found the issue! The login function is checking against a hardcoded password instead of verifying against stored credentials. This is a security vulnerability.\n\nI'll fix it to use proper password hashing.".to_string(),
                    word_delay_ms: 30,
                },
                DemoStep::Pause(Duration::from_millis(600)),
                // Write fix
                DemoStep::ToolRequest {
                    name: "write_file".to_string(),
                    description: "Modify file contents".to_string(),
                    args: vec![
                        ("path".to_string(), "src/auth.rs".to_string()),
                        ("changes".to_string(), "+12 lines, -2 lines".to_string()),
                    ],
                    risk: ToolRisk::Medium,
                },
                DemoStep::Pause(Duration::from_millis(1200)),
                DemoStep::UserApproves {
                    choice: ApprovalChoice::AllowOnce,
                },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(400),
                    output: None,
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::AgentStreams {
                    text: "Fixed! The login function now:\n\n1. Fetches the user from the database\n2. Verifies the password using bcrypt\n3. Returns a proper Result type\n\nWould you like me to add tests for this?".to_string(),
                    word_delay_ms: 28,
                },
                DemoStep::Pause(Duration::from_secs(3)),
                DemoStep::SystemMessage("Demo complete.".to_string()),
            ],
        }
    }

    /// Ultimate showcase - demonstrates ALL UI capabilities
    #[allow(clippy::too_many_lines)]
    fn showcase() -> Self {
        Self {
            name: "showcase".to_string(),
            description: "Ultimate demo: boot sequence, all views, every feature".to_string(),
            steps: vec![
                // ═══════════════════════════════════════════════════════════════
                // ACT 1: BOOT SEQUENCE - Stars twinkle, logo appears
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 1: Boot Sequence".to_string()),
                DemoStep::BootSequence {
                    cinematic: true,
                    checks: vec![
                        ("Runtime initialized".to_string(), true),
                        ("Gateway connected".to_string(), true),
                        ("Workspace loaded: astralis-demo".to_string(), true),
                        ("MCP servers: 3 online".to_string(), true),
                    ],
                },
                DemoStep::Pause(Duration::from_millis(1200)),
                DemoStep::Clear, // Transition from boot to main UI

                // ═══════════════════════════════════════════════════════════════
                // ACT 2: NEXUS VIEW - Conversation with tool calls
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 2: Comms - Conversation Hub".to_string()),
                DemoStep::SwitchView(View::Nexus),
                DemoStep::Pause(Duration::from_millis(750)),

                // User asks a question
                DemoStep::UserTypes {
                    text: "Help me fix the authentication bug in src/auth.rs".to_string(),
                    typing_speed_ms: 60,
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::UserSubmits,

                // Agent thinks
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(1800),
                },

                // Agent responds and requests tool
                DemoStep::AgentStreams {
                    text: "I'll investigate the auth module. Let me read the file first.".to_string(),
                    word_delay_ms: 52,
                },
                DemoStep::Pause(Duration::from_millis(600)),

                // Tool request - low risk
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "src/auth.rs".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(1500)),
                DemoStep::UserApproves { choice: ApprovalChoice::AllowSession },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(450),
                    output: Some("pub fn login(user: &str, pass: &str) -> bool {\n    pass == \"admin\"\n}".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(750)),

                // Agent explains the issue
                DemoStep::AgentStreams {
                    text: "Found it! The password is hardcoded. This is a critical security flaw.".to_string(),
                    word_delay_ms: 45,
                },
                DemoStep::Pause(Duration::from_millis(600)),

                // Show diff
                DemoStep::ShowDiff {
                    file: "src/auth.rs".to_string(),
                    removed: vec!["    pass == \"admin\"".to_string()],
                    added: vec![
                        "    let user = db.find_user(user)?;".to_string(),
                        "    bcrypt::verify(pass, &user.password_hash)".to_string(),
                    ],
                },
                DemoStep::Pause(Duration::from_millis(1200)),

                // Write file - medium risk
                DemoStep::ToolRequest {
                    name: "write_file".to_string(),
                    description: "Modify file contents".to_string(),
                    args: vec![
                        ("path".to_string(), "src/auth.rs".to_string()),
                        ("changes".to_string(), "+8 -1".to_string()),
                    ],
                    risk: ToolRisk::Medium,
                },
                DemoStep::Pause(Duration::from_millis(1800)),
                DemoStep::UserApproves { choice: ApprovalChoice::AllowOnce },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(600),
                    output: None,
                    success: true,
                },

                DemoStep::AgentStreams {
                    text: "Fixed! The auth module now uses bcrypt for secure password verification.".to_string(),
                    word_delay_ms: 45,
                },
                DemoStep::Pause(Duration::from_millis(1500)),

                // ═══════════════════════════════════════════════════════════════
                // ACT 3: SIDEBAR COLLAPSE
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 3: Sidebar States".to_string()),
                DemoStep::Pause(Duration::from_millis(750)),
                DemoStep::ToggleSidebar(SidebarState::Collapsed),
                DemoStep::Pause(Duration::from_millis(1200)),
                DemoStep::ToggleSidebar(SidebarState::Expanded),
                DemoStep::Pause(Duration::from_millis(750)),

                // ═══════════════════════════════════════════════════════════════
                // ACT 4: MISSIONS VIEW - Task board
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Clear,
                DemoStep::Narrate("Act 4: Missions - Task Tracker".to_string()),
                DemoStep::SwitchView(View::Missions),
                DemoStep::Pause(Duration::from_millis(900)),

                // Add some tasks
                DemoStep::AddTask {
                    id: "auth-fix".to_string(),
                    title: "Fix authentication".to_string(),
                    status: TaskStatus::Complete,
                },
                DemoStep::Pause(Duration::from_millis(450)),
                DemoStep::AddTask {
                    id: "add-tests".to_string(),
                    title: "Add unit tests".to_string(),
                    status: TaskStatus::Active,
                },
                DemoStep::Pause(Duration::from_millis(450)),
                DemoStep::AddTask {
                    id: "docs".to_string(),
                    title: "Update documentation".to_string(),
                    status: TaskStatus::Backlog,
                },
                DemoStep::Pause(Duration::from_millis(450)),
                DemoStep::AddTask {
                    id: "review".to_string(),
                    title: "Code review".to_string(),
                    status: TaskStatus::Backlog,
                },
                DemoStep::Pause(Duration::from_millis(1200)),

                // Move a task
                DemoStep::MoveTask {
                    id: "add-tests".to_string(),
                    status: TaskStatus::Complete,
                },
                DemoStep::Pause(Duration::from_millis(900)),
                DemoStep::MoveTask {
                    id: "docs".to_string(),
                    status: TaskStatus::Active,
                },
                DemoStep::Pause(Duration::from_millis(1500)),

                // ═══════════════════════════════════════════════════════════════
                // ACT 5: STELLAR VIEW - File explorer
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Clear,
                DemoStep::Narrate("Act 5: Atlas - File Explorer".to_string()),
                DemoStep::SwitchView(View::Stellar),
                DemoStep::Pause(Duration::from_millis(900)),

                // Show files with various states
                DemoStep::ShowFile {
                    path: "src/main.rs".to_string(),
                    status: FileStatus::Unchanged,
                },
                DemoStep::ShowFile {
                    path: "src/auth.rs".to_string(),
                    status: FileStatus::Modified,
                },
                DemoStep::ShowFile {
                    path: "src/lib.rs".to_string(),
                    status: FileStatus::Unchanged,
                },
                DemoStep::ShowFile {
                    path: "tests/auth_test.rs".to_string(),
                    status: FileStatus::Added,
                },
                DemoStep::ShowFile {
                    path: "Cargo.toml".to_string(),
                    status: FileStatus::Modified,
                },
                DemoStep::Pause(Duration::from_millis(1500)),

                // ═══════════════════════════════════════════════════════════════
                // ACT 6: LOG VIEW - Minimal mode
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Clear,
                DemoStep::Narrate("Act 6: Console - Minimal Mode".to_string()),
                DemoStep::SwitchView(View::Log),
                DemoStep::ToggleSidebar(SidebarState::Hidden),
                DemoStep::Pause(Duration::from_millis(900)),

                // Quick interaction in minimal mode
                DemoStep::UserTypes {
                    text: "Run the tests".to_string(),
                    typing_speed_ms: 75,
                },
                DemoStep::UserSubmits,
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(1200),
                },
                DemoStep::AgentStreams {
                    text: "Running test suite...".to_string(),
                    word_delay_ms: 60,
                },
                DemoStep::Pause(Duration::from_millis(450)),

                // Tool in minimal mode (inline approval)
                DemoStep::ToolRequest {
                    name: "bash".to_string(),
                    description: "Run shell command".to_string(),
                    args: vec![("cmd".to_string(), "cargo test".to_string())],
                    risk: ToolRisk::Medium,
                },
                DemoStep::Pause(Duration::from_millis(1200)),
                DemoStep::UserApproves { choice: ApprovalChoice::AllowOnce },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(2250),
                    output: Some("running 4 tests\ntest auth::test_login ... ok\ntest auth::test_logout ... ok\ntest auth::test_hash ... ok\ntest auth::test_verify ... ok\n\ntest result: ok. 4 passed".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(750)),

                DemoStep::AgentStreams {
                    text: "All 4 tests passing. The authentication fix is verified.".to_string(),
                    word_delay_ms: 45,
                },
                DemoStep::Pause(Duration::from_millis(1500)),

                // ═══════════════════════════════════════════════════════════════
                // ACT 8: APPROVAL VARIANTS
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Clear,
                DemoStep::Narrate("Act 7: Approval Options".to_string()),
                DemoStep::SwitchView(View::Nexus),
                DemoStep::ToggleSidebar(SidebarState::Expanded),
                DemoStep::Pause(Duration::from_millis(750)),

                // High risk - deny
                DemoStep::UserTypes {
                    text: "Delete all temporary files".to_string(),
                    typing_speed_ms: 68,
                },
                DemoStep::UserSubmits,
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(900),
                },
                DemoStep::ToolRequest {
                    name: "delete_files".to_string(),
                    description: "Remove files matching pattern".to_string(),
                    args: vec![
                        ("pattern".to_string(), "**/*.tmp".to_string()),
                        ("recursive".to_string(), "true".to_string()),
                    ],
                    risk: ToolRisk::High,
                },
                DemoStep::Pause(Duration::from_millis(2250)),
                DemoStep::UserApproves { choice: ApprovalChoice::Deny },
                DemoStep::Pause(Duration::from_millis(600)),

                DemoStep::AgentStreams {
                    text: "Understood. I won't delete those files. Would you like me to list them first so you can review?".to_string(),
                    word_delay_ms: 45,
                },
                DemoStep::Pause(Duration::from_millis(1500)),

                // ═══════════════════════════════════════════════════════════════
                // ACT 9: ERROR RECOVERY
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 8: Error Recovery".to_string()),
                DemoStep::UserTypes {
                    text: "Read the config file".to_string(),
                    typing_speed_ms: 75,
                },
                DemoStep::UserSubmits,
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(750),
                },
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "config.yaml".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(900)),
                DemoStep::UserApproves { choice: ApprovalChoice::AllowOnce },

                // Tool fails
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(450),
                    output: Some("Error: File not found".to_string()),
                    success: false,
                },
                DemoStep::Pause(Duration::from_millis(750)),

                DemoStep::AgentStreams {
                    text: "The file doesn't exist at that path. Let me check alternative locations...".to_string(),
                    word_delay_ms: 45,
                },
                DemoStep::Pause(Duration::from_millis(600)),

                // Retry with different path
                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), ".config/settings.yaml".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(750)),
                DemoStep::SystemMessage("Auto-approved (session permission)".to_string()),
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(375),
                    output: Some("database:\n  host: localhost\n  port: 5432".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(600)),

                DemoStep::AgentStreams {
                    text: "Found it at `.config/settings.yaml`. Your database is configured for localhost:5432.".to_string(),
                    word_delay_ms: 42,
                },
                DemoStep::Pause(Duration::from_millis(1500)),

                // ═══════════════════════════════════════════════════════════════
                // FINALE
                // ═══════════════════════════════════════════════════════════════
                DemoStep::Narrate("✧ Showcase Complete ✧".to_string()),
                DemoStep::Pause(Duration::from_millis(750)),
                DemoStep::OrbitStatus("* Demo complete — all features demonstrated".to_string()),
                DemoStep::Pause(Duration::from_secs(3)),

                DemoStep::SystemMessage(String::new()),
                DemoStep::SystemMessage("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string()),
                DemoStep::SystemMessage("  ✧ A S T R A L I S ✧".to_string()),
                DemoStep::SystemMessage("  Demonstrated:".to_string()),
                DemoStep::SystemMessage("    * Boot sequence with twinkling stars".to_string()),
                DemoStep::SystemMessage("    * Nexus: Conversation with tool calls".to_string()),
                DemoStep::SystemMessage("    * Missions: Task board management".to_string()),
                DemoStep::SystemMessage("    * Atlas: File explorer with status".to_string()),
                DemoStep::SystemMessage("    * Console: Minimal mode".to_string()),
                DemoStep::SystemMessage("    * Sidebar: Expand/collapse/hide".to_string()),
                DemoStep::SystemMessage("    * Approvals: Allow/Always/Session/Deny".to_string()),
                DemoStep::SystemMessage("    * Error recovery".to_string()),
                DemoStep::SystemMessage("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string()),
                DemoStep::Pause(Duration::from_millis(7500)),
            ],
        }
    }

    /// Quick ~30 second demo hitting key features
    fn quick() -> Self {
        Self {
            name: "quick".to_string(),
            description: "Quick demo: boot, conversation, tool call, all views".to_string(),
            steps: vec![
                // Boot
                DemoStep::BootSequence {
                    cinematic: false,
                    checks: vec![
                        ("Runtime initialized".to_string(), true),
                        ("Workspace loaded".to_string(), true),
                    ],
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::Clear,

                // Quick conversation with tool call
                DemoStep::UserTypes {
                    text: "What does src/main.rs do?".to_string(),
                    typing_speed_ms: 30,
                },
                DemoStep::UserSubmits,
                DemoStep::AgentThinking {
                    duration: Duration::from_millis(800),
                },
                DemoStep::AgentStreams {
                    text: "Let me read the file to explain it.".to_string(),
                    word_delay_ms: 25,
                },
                DemoStep::Pause(Duration::from_millis(200)),

                DemoStep::ToolRequest {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                    args: vec![("path".to_string(), "src/main.rs".to_string())],
                    risk: ToolRisk::Low,
                },
                DemoStep::Pause(Duration::from_millis(600)),
                DemoStep::UserApproves { choice: ApprovalChoice::AllowOnce },
                DemoStep::ToolExecutes {
                    duration: Duration::from_millis(200),
                    output: Some("fn main() {\n    let app = App::new();\n    app.run();\n}".to_string()),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(300)),

                DemoStep::AgentStreams {
                    text: "The **main function** creates an `App` instance and runs it. Simple entry point.".to_string(),
                    word_delay_ms: 25,
                },
                DemoStep::Pause(Duration::from_millis(500)),

                // Quick tour of views
                DemoStep::SwitchView(View::Missions),
                DemoStep::AddTask {
                    id: "t1".to_string(),
                    title: "Read main.rs".to_string(),
                    status: TaskStatus::Complete,
                },
                DemoStep::AddTask {
                    id: "t2".to_string(),
                    title: "Explain code".to_string(),
                    status: TaskStatus::Active,
                },
                DemoStep::Pause(Duration::from_millis(800)),

                DemoStep::SwitchView(View::Stellar),
                DemoStep::ShowFile {
                    path: "src/main.rs".to_string(),
                    status: FileStatus::Unchanged,
                },
                DemoStep::ShowFile {
                    path: "src/lib.rs".to_string(),
                    status: FileStatus::Modified,
                },
                DemoStep::Pause(Duration::from_millis(800)),

                DemoStep::SwitchView(View::Log),
                DemoStep::ToggleSidebar(SidebarState::Hidden),
                DemoStep::Pause(Duration::from_millis(600)),

                // Back to nexus
                DemoStep::SwitchView(View::Nexus),
                DemoStep::ToggleSidebar(SidebarState::Expanded),
                DemoStep::Pause(Duration::from_secs(2)),
            ],
        }
    }

    /// Multi-agent operations demo - demonstrates all new views
    #[allow(clippy::too_many_lines)]
    fn multi_agent_ops() -> Self {
        Self {
            name: "multi-agent-ops".to_string(),
            description: "Multi-agent: 3 agents, sub-agents, approvals, audit chain".to_string(),
            steps: vec![
                // ═══════════════════════════════════════════════════════════
                // ACT 1: BOOT + SPAWN AGENTS
                // ═══════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 1: Multi-Agent Boot".to_string()),
                DemoStep::BootSequence {
                    cinematic: true,
                    checks: vec![
                        ("Runtime initialized".to_string(), true),
                        ("Gateway connected".to_string(), true),
                        ("Agent pool: 3 slots".to_string(), true),
                        ("MCP servers: 4 online".to_string(), true),
                    ],
                },
                DemoStep::Pause(Duration::from_millis(800)),
                DemoStep::Clear,
                // Spawn 3 agents
                DemoStep::SpawnAgent {
                    name: "alpha".to_string(),
                    model: "claude-opus-4.6".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::SpawnAgent {
                    name: "beta".to_string(),
                    model: "claude-sonnet-4.5".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::SpawnAgent {
                    name: "gamma".to_string(),
                    model: "claude-haiku-4.5".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(500)),
                // ═══════════════════════════════════════════════════════════
                // ACT 2: COMMAND CENTER
                // ═══════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 2: Command Center".to_string()),
                DemoStep::SwitchView(View::Command),
                DemoStep::Pause(Duration::from_millis(1000)),
                // Agents start working
                DemoStep::SetAgentStatus {
                    agent: "alpha".to_string(),
                    status: AgentStatusDemo::Busy,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::SetAgentStatus {
                    agent: "beta".to_string(),
                    status: AgentStatusDemo::Ready,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::SetAgentStatus {
                    agent: "gamma".to_string(),
                    status: AgentStatusDemo::Busy,
                },
                DemoStep::Pause(Duration::from_millis(600)),
                // Events flow in
                DemoStep::AddEventRecord {
                    agent: "alpha".to_string(),
                    event_type: "McpToolCalled".to_string(),
                    detail: "read_file(src/auth.rs)".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::AddEventRecord {
                    agent: "gamma".to_string(),
                    event_type: "McpToolCalled".to_string(),
                    detail: "search_code(\"password\")".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(300)),
                DemoStep::AddEventRecord {
                    agent: "alpha".to_string(),
                    event_type: "CapabilityGranted".to_string(),
                    detail: "mcp://fs:read_file session".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(1500)),
                // ═══════════════════════════════════════════════════════════
                // ACT 3: TOPOLOGY - Sub-agent delegation
                // ═══════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 3: Topology - Sub-Agent Delegation".to_string()),
                DemoStep::SwitchView(View::Topology),
                DemoStep::Pause(Duration::from_millis(800)),
                // Alpha delegates sub-tasks
                DemoStep::SpawnSubAgent {
                    parent_agent: "alpha".to_string(),
                    task: "Analyze auth patterns".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(500)),
                DemoStep::SpawnSubAgent {
                    parent_agent: "alpha".to_string(),
                    task: "Read config files".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(500)),
                // Gamma delegates too
                DemoStep::SpawnSubAgent {
                    parent_agent: "gamma".to_string(),
                    task: "Deploy to staging".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(1000)),
                // One completes
                DemoStep::CompleteSubAgent {
                    id: "sub-002".to_string(),
                    success: true,
                },
                DemoStep::Pause(Duration::from_millis(800)),
                // One fails
                DemoStep::CompleteSubAgent {
                    id: "sub-003".to_string(),
                    success: false,
                },
                DemoStep::Pause(Duration::from_millis(800)),
                DemoStep::SetAgentStatus {
                    agent: "gamma".to_string(),
                    status: AgentStatusDemo::Error,
                },
                DemoStep::Pause(Duration::from_millis(1200)),
                // ═══════════════════════════════════════════════════════════
                // ACT 4: SHIELD - Security Dashboard
                // ═══════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 4: Shield - Security Dashboard".to_string()),
                DemoStep::SwitchView(View::Shield),
                DemoStep::Pause(Duration::from_millis(800)),
                // Grant capabilities
                DemoStep::GrantCapability {
                    agent: "alpha".to_string(),
                    resource: "mcp://fs:read_*".to_string(),
                    scope: "session".to_string(),
                    ttl_secs: Some(7200),
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::GrantCapability {
                    agent: "alpha".to_string(),
                    resource: "mcp://fs:write_*".to_string(),
                    scope: "persistent".to_string(),
                    ttl_secs: None,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Approval requests
                DemoStep::AddShieldApproval {
                    agent: "alpha".to_string(),
                    tool: "delete_files(**/*.tmp)".to_string(),
                    risk: ToolRisk::High,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                DemoStep::AddShieldApproval {
                    agent: "gamma".to_string(),
                    tool: "execute_cmd(deploy.sh)".to_string(),
                    risk: ToolRisk::High,
                },
                DemoStep::Pause(Duration::from_millis(800)),
                // Security violation
                DemoStep::SecurityViolation {
                    agent: "gamma".to_string(),
                    detail: "Workspace escape attempt: /etc/passwd".to_string(),
                },
                DemoStep::Pause(Duration::from_millis(600)),
                DemoStep::SetThreatLevel(ThreatLevelDemo::Elevated),
                DemoStep::Pause(Duration::from_millis(1500)),
                // ═══════════════════════════════════════════════════════════
                // ACT 5: PULSE - Health & Budget
                // ═══════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 5: Pulse - Health & Budget".to_string()),
                DemoStep::SwitchView(View::Pulse),
                DemoStep::Pause(Duration::from_millis(800)),
                // Set health checks
                DemoStep::SetHealth {
                    component: "agent_manager".to_string(),
                    status: HealthStatusDemo::Ok,
                },
                DemoStep::SetHealth {
                    component: "mcp_pool".to_string(),
                    status: HealthStatusDemo::Ok,
                },
                DemoStep::SetHealth {
                    component: "approval_queue".to_string(),
                    status: HealthStatusDemo::Ok,
                },
                DemoStep::SetHealth {
                    component: "audit_log".to_string(),
                    status: HealthStatusDemo::Degraded,
                },
                DemoStep::Pause(Duration::from_millis(400)),
                // Set budget data
                DemoStep::SetBudget {
                    agent: "alpha".to_string(),
                    spent: 1.43,
                },
                DemoStep::SetBudget {
                    agent: "beta".to_string(),
                    spent: 0.00,
                },
                DemoStep::SetBudget {
                    agent: "gamma".to_string(),
                    spent: 0.71,
                },
                DemoStep::Pause(Duration::from_millis(1500)),
                // ═══════════════════════════════════════════════════════════
                // ACT 6: CHAIN - Audit Trail
                // ═══════════════════════════════════════════════════════════
                DemoStep::Narrate("Act 6: Chain - Audit Trail".to_string()),
                DemoStep::SwitchView(View::Chain),
                DemoStep::Pause(Duration::from_millis(800)),
                // Add audit entries
                DemoStep::AddAuditEntry {
                    agent: "alpha".to_string(),
                    action: "SessionStart".to_string(),
                    outcome: AuditOutcomeDemo::Success,
                },
                DemoStep::Pause(Duration::from_millis(200)),
                DemoStep::AddAuditEntry {
                    agent: "alpha".to_string(),
                    action: "McpToolCall".to_string(),
                    outcome: AuditOutcomeDemo::Success,
                },
                DemoStep::Pause(Duration::from_millis(200)),
                DemoStep::AddAuditEntry {
                    agent: "alpha".to_string(),
                    action: "CapabilityGrant".to_string(),
                    outcome: AuditOutcomeDemo::Success,
                },
                DemoStep::Pause(Duration::from_millis(200)),
                DemoStep::AddAuditEntry {
                    agent: "gamma".to_string(),
                    action: "McpToolCall".to_string(),
                    outcome: AuditOutcomeDemo::Success,
                },
                DemoStep::Pause(Duration::from_millis(200)),
                DemoStep::AddAuditEntry {
                    agent: "gamma".to_string(),
                    action: "SecurityViolation".to_string(),
                    outcome: AuditOutcomeDemo::Violation,
                },
                DemoStep::Pause(Duration::from_millis(200)),
                DemoStep::AddAuditEntry {
                    agent: "alpha".to_string(),
                    action: "ApprovalRequested".to_string(),
                    outcome: AuditOutcomeDemo::Success,
                },
                DemoStep::Pause(Duration::from_millis(1500)),
                // ═══════════════════════════════════════════════════════════
                // FINALE
                // ═══════════════════════════════════════════════════════════
                DemoStep::Narrate("✧ Multi-Agent Demo Complete ✧".to_string()),
                DemoStep::Pause(Duration::from_millis(750)),
                DemoStep::SwitchView(View::Command),
                DemoStep::Pause(Duration::from_millis(500)),
                DemoStep::OrbitStatus("* Demo complete — all agent views demonstrated".to_string()),
                DemoStep::Pause(Duration::from_secs(3)),
                DemoStep::SystemMessage(String::new()),
                DemoStep::SystemMessage("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string()),
                DemoStep::SystemMessage("  ✧ A S T R A L I S ✧".to_string()),
                DemoStep::SystemMessage("  Multi-Agent Demo:".to_string()),
                DemoStep::SystemMessage("    * Command: Agent grid overhead".to_string()),
                DemoStep::SystemMessage("    * Topology: Sub-agent tree".to_string()),
                DemoStep::SystemMessage("    * Shield: Security dashboard".to_string()),
                DemoStep::SystemMessage("    * Pulse: Health & budget".to_string()),
                DemoStep::SystemMessage("    * Chain: Audit trail".to_string()),
                DemoStep::SystemMessage("    * 3 agents, sub-delegation".to_string()),
                DemoStep::SystemMessage("    * Capability tokens, threat levels".to_string()),
                DemoStep::SystemMessage("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string()),
                DemoStep::Pause(Duration::from_millis(7500)),
            ],
        }
    }
}
