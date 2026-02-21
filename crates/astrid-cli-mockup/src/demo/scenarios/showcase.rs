use super::{
    ApprovalChoice, DemoScenario, DemoStep, FileStatus, SidebarState, TaskStatus, ToolRisk, View,
};
use std::time::Duration;

#[allow(clippy::too_many_lines)]
pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
                    ("Workspace loaded: astrid-demo".to_string(), true),
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
