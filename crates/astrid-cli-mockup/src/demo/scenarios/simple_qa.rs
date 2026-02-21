use super::{DemoScenario, DemoStep};
use std::time::Duration;

pub(super) fn build() -> DemoScenario {
    DemoScenario {
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
