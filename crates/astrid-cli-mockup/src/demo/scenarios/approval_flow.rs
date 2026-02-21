use std::time::Duration;
use super::{
    DemoScenario, DemoStep, ToolRisk, ApprovalChoice, View, 
    NexusCategoryDemo, SidebarState, TaskStatus, FileStatus,
    AgentStatusDemo, AuditOutcomeDemo, HealthStatusDemo, ThreatLevelDemo
};

pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
