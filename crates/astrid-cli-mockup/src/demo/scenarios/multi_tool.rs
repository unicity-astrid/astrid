use std::time::Duration;
use super::{
    DemoScenario, DemoStep, ToolRisk, ApprovalChoice, View, 
    NexusCategoryDemo, SidebarState, TaskStatus, FileStatus,
    AgentStatusDemo, AuditOutcomeDemo, HealthStatusDemo, ThreatLevelDemo
};

pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
