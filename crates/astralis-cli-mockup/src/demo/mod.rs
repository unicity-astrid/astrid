//! Demo scenarios for showcasing the UI.
//!
//! Demos are fully scripted playback - like watching a movie of the experience.
//! No actual user input required during playback.

mod player;
mod scenarios;

pub(crate) use player::DemoPlayer;
pub(crate) use scenarios::DemoScenario;

// Re-export types for potential external use
#[allow(unused_imports)]
pub(crate) use scenarios::{
    AgentStatusDemo, ApprovalChoice, AuditOutcomeDemo, DemoStep, FileStatus, HealthStatusDemo,
    SidebarState, TaskStatus, ThreatLevelDemo, ToolRisk, View,
};
