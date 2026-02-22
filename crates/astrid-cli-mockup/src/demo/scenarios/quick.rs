use super::{
    ApprovalChoice, DemoScenario, DemoStep, FileStatus, SidebarState, TaskStatus, ToolRisk, View,
};
use std::time::Duration;

pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
