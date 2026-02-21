use std::time::Duration;
use super::{
    DemoScenario, DemoStep, ToolRisk, ApprovalChoice, View, 
    NexusCategoryDemo, SidebarState, TaskStatus, FileStatus,
    AgentStatusDemo, AuditOutcomeDemo, HealthStatusDemo, ThreatLevelDemo
};

pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
