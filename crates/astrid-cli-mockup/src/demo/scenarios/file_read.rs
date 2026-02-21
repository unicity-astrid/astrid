use std::time::Duration;
use super::{
    DemoScenario, DemoStep, ToolRisk, ApprovalChoice, View, 
    NexusCategoryDemo, SidebarState, TaskStatus, FileStatus,
    AgentStatusDemo, AuditOutcomeDemo, HealthStatusDemo, ThreatLevelDemo
};

pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
