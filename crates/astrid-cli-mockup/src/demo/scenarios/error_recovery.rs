use super::{
    ApprovalChoice, DemoScenario, DemoStep, ToolRisk,
};
use std::time::Duration;

pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
