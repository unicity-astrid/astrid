use super::{ApprovalChoice, DemoScenario, DemoStep, ToolRisk};
use std::time::Duration;

pub(super) fn build() -> DemoScenario {
    DemoScenario {
        name: "full-demo".to_string(),
        description: "Complete end-to-end showcase of all features".to_string(),
        steps: vec![
            DemoStep::SystemMessage("Astrid Interactive CLI Demo".to_string()),
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
