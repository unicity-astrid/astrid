//! Hook profiles - predefined hook configurations.

use serde::{Deserialize, Serialize};

use crate::hook::{FailAction, Hook, HookEvent, HookHandler};

/// A profile containing a set of hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookProfile {
    /// Profile name.
    pub name: String,
    /// Profile description.
    pub description: String,
    /// Hooks in this profile.
    pub hooks: Vec<Hook>,
}

impl HookProfile {
    /// Create a new hook profile.
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            hooks: Vec::new(),
        }
    }

    /// Add a hook to the profile.
    #[must_use]
    pub fn with_hook(mut self, hook: Hook) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Create a minimal profile with no hooks.
    #[must_use]
    pub fn minimal() -> Self {
        Self::new("minimal", "Minimal profile with no hooks")
    }

    /// Create a logging profile that logs all major events.
    #[must_use]
    pub fn logging() -> Self {
        Self::new("logging", "Profile that logs all major events to stdout")
            .with_hook(
                Hook::new(HookEvent::SessionStart)
                    .with_name("log-session-start")
                    .with_handler(HookHandler::Command {
                        command: "echo".to_string(),
                        args: vec!["[ASTRID] Session started: $ASTRID_SESSION_ID".to_string()],
                        env: std::collections::HashMap::new(),
                        working_dir: None,
                    })
                    .with_fail_action(FailAction::Ignore)
                    .async_mode(),
            )
            .with_hook(
                Hook::new(HookEvent::SessionEnd)
                    .with_name("log-session-end")
                    .with_handler(HookHandler::Command {
                        command: "echo".to_string(),
                        args: vec!["[ASTRID] Session ended: $ASTRID_SESSION_ID".to_string()],
                        env: std::collections::HashMap::new(),
                        working_dir: None,
                    })
                    .with_fail_action(FailAction::Ignore)
                    .async_mode(),
            )
            .with_hook(
                Hook::new(HookEvent::PreToolCall)
                    .with_name("log-tool-call")
                    .with_handler(HookHandler::Command {
                        command: "echo".to_string(),
                        args: vec!["[ASTRID] Tool call: $ASTRID_HOOK_DATA".to_string()],
                        env: std::collections::HashMap::new(),
                        working_dir: None,
                    })
                    .with_fail_action(FailAction::Ignore)
                    .async_mode(),
            )
    }

    /// Create a security profile that blocks dangerous operations.
    #[must_use]
    pub fn security() -> Self {
        Self::new(
            "security",
            "Profile that adds security checks before sensitive operations",
        )
        .with_hook(
            Hook::new(HookEvent::PreToolCall)
                .with_name("block-dangerous-tools")
                .with_description("Block execution of potentially dangerous tools")
                .with_handler(HookHandler::Command {
                    command: "sh".to_string(),
                    args: vec![
                        "-c".to_string(),
                        r#"
                        TOOL=$(echo "$ASTRID_HOOK_DATA" | grep -o '"tool_name":"[^"]*"' | cut -d'"' -f4)
                        case "$TOOL" in
                            rm|sudo|chmod|chown|mkfs|dd)
                                echo "block: Dangerous tool '$TOOL' blocked by security policy"
                                ;;
                            *)
                                echo "continue"
                                ;;
                        esac
                        "#
                        .to_string(),
                    ],
                    env: std::collections::HashMap::new(),
                    working_dir: None,
                })
                .with_fail_action(FailAction::Block)
                .with_timeout(5),
        )
    }

    /// Create a notification profile that sends webhooks.
    #[must_use]
    pub fn notifications(webhook_url: impl Into<String>) -> Self {
        let url = webhook_url.into();

        Self::new(
            "notifications",
            "Profile that sends webhook notifications for events",
        )
        .with_hook(
            Hook::new(HookEvent::SessionStart)
                .with_name("notify-session-start")
                .with_handler(HookHandler::Http {
                    url: url.clone(),
                    method: "POST".to_string(),
                    headers: std::collections::HashMap::new(),
                    body_template: Some(
                        r#"{"event": "session_start", "session_id": "{{session_id}}"}"#.to_string(),
                    ),
                })
                .with_fail_action(FailAction::Warn)
                .async_mode(),
        )
        .with_hook(
            Hook::new(HookEvent::SessionEnd)
                .with_name("notify-session-end")
                .with_handler(HookHandler::Http {
                    url,
                    method: "POST".to_string(),
                    headers: std::collections::HashMap::new(),
                    body_template: Some(
                        r#"{"event": "session_end", "session_id": "{{session_id}}"}"#.to_string(),
                    ),
                })
                .with_fail_action(FailAction::Warn)
                .async_mode(),
        )
    }

    /// Create a development profile with helpful debugging hooks.
    #[must_use]
    pub fn development() -> Self {
        Self::new(
            "development",
            "Profile for development with debugging helpers",
        )
        .with_hook(
            Hook::new(HookEvent::PreToolCall)
                .with_name("debug-tool-calls")
                .with_handler(HookHandler::Command {
                    command: "sh".to_string(),
                    args: vec![
                        "-c".to_string(),
                        r#"echo "[DEBUG] Tool call at $(date): $ASTRID_HOOK_DATA" >> /tmp/astrid-debug.log"#.to_string(),
                    ],
                    env: std::collections::HashMap::new(),
                    working_dir: None,
                })
                .with_fail_action(FailAction::Ignore)
                .async_mode(),
        )
        .with_hook(
            Hook::new(HookEvent::ToolError)
                .with_name("debug-tool-errors")
                .with_handler(HookHandler::Command {
                    command: "sh".to_string(),
                    args: vec![
                        "-c".to_string(),
                        r#"echo "[ERROR] Tool error at $(date): $ASTRID_HOOK_DATA" >> /tmp/astrid-debug.log"#.to_string(),
                    ],
                    env: std::collections::HashMap::new(),
                    working_dir: None,
                })
                .with_fail_action(FailAction::Ignore)
                .async_mode(),
        )
    }
}

/// Get a profile by name.
#[must_use]
pub fn get_profile(name: &str) -> Option<HookProfile> {
    match name {
        "minimal" => Some(HookProfile::minimal()),
        "logging" => Some(HookProfile::logging()),
        "security" => Some(HookProfile::security()),
        "development" => Some(HookProfile::development()),
        _ => None,
    }
}

/// List available built-in profile names.
#[must_use]
pub fn available_profiles() -> Vec<&'static str> {
    vec!["minimal", "logging", "security", "development"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimal_profile() {
        let profile = HookProfile::minimal();
        assert_eq!(profile.name, "minimal");
        assert!(profile.hooks.is_empty());
    }

    #[test]
    fn test_logging_profile() {
        let profile = HookProfile::logging();
        assert_eq!(profile.name, "logging");
        assert!(!profile.hooks.is_empty());

        // Should have session start, session end, and tool call hooks
        let events: Vec<_> = profile.hooks.iter().map(|h| h.event).collect();
        assert!(events.contains(&HookEvent::SessionStart));
        assert!(events.contains(&HookEvent::SessionEnd));
        assert!(events.contains(&HookEvent::PreToolCall));
    }

    #[test]
    fn test_security_profile() {
        let profile = HookProfile::security();
        assert_eq!(profile.name, "security");

        // Should have a blocking hook
        assert!(
            profile
                .hooks
                .iter()
                .any(|h| h.fail_action == FailAction::Block)
        );
    }

    #[test]
    fn test_notifications_profile() {
        let profile = HookProfile::notifications("https://example.com/webhook");
        assert_eq!(profile.name, "notifications");

        // Should have HTTP handlers
        assert!(
            profile
                .hooks
                .iter()
                .all(|h| matches!(h.handler, HookHandler::Http { .. }))
        );
    }

    #[test]
    fn test_get_profile() {
        assert!(get_profile("minimal").is_some());
        assert!(get_profile("logging").is_some());
        assert!(get_profile("security").is_some());
        assert!(get_profile("unknown").is_none());
    }

    #[test]
    fn test_available_profiles() {
        let profiles = available_profiles();
        assert!(profiles.contains(&"minimal"));
        assert!(profiles.contains(&"logging"));
        assert!(profiles.contains(&"security"));
    }
}
