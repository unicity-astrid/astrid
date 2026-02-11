//! Workspace profiles - predefined workspace configurations.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::{EscapePolicy, WorkspaceConfig, WorkspaceMode};

/// A workspace profile with predefined settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProfile {
    /// Profile name.
    pub name: String,
    /// Profile description.
    pub description: String,
    /// Configuration for this profile.
    pub config: WorkspaceConfig,
}

impl WorkspaceProfile {
    /// Create a new workspace profile.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        config: WorkspaceConfig,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            config,
        }
    }

    /// Create a "safe" profile - maximum restrictions.
    ///
    /// - Safe mode: always ask before leaving workspace
    /// - No auto-allowed paths outside workspace
    /// - Standard protected paths
    #[must_use]
    pub fn safe(root: impl Into<PathBuf>) -> Self {
        let config = WorkspaceConfig::new(root)
            .with_mode(WorkspaceMode::Safe)
            .with_escape_policy(EscapePolicy::Ask);

        Self::new(
            "safe",
            "Maximum restrictions - always ask before leaving workspace",
            config,
        )
    }

    /// Create a "power user" profile - balanced restrictions.
    ///
    /// - Guided mode: smart defaults
    /// - Auto-allow common development paths
    /// - Standard protected paths
    #[must_use]
    pub fn power_user(root: impl Into<PathBuf>) -> Self {
        let root = root.into();

        // Auto-allow common development locations
        let config = WorkspaceConfig::new(&root)
            .with_mode(WorkspaceMode::Guided)
            .with_escape_policy(EscapePolicy::Ask)
            // Common read-only paths for development
            .allow_read("/usr/local/include")
            .allow_read("/usr/include")
            .allow_read("/opt")
            // User's home directory common locations
            .allow_read(dirs_home().map(|h| h.join(".cargo")).unwrap_or_default())
            .allow_read(dirs_home().map(|h| h.join(".rustup")).unwrap_or_default())
            .allow_read(dirs_home().map(|h| h.join(".npm")).unwrap_or_default())
            .allow_read(dirs_home().map(|h| h.join(".config")).unwrap_or_default());

        Self::new(
            "power_user",
            "Balanced restrictions - auto-allow common development paths",
            config,
        )
    }

    /// Create an "autonomous" profile - minimal restrictions.
    ///
    /// - Autonomous mode: no restrictions
    /// - All paths allowed except protected system paths
    /// - Use with caution!
    #[must_use]
    pub fn autonomous(root: impl Into<PathBuf>) -> Self {
        let config = WorkspaceConfig::new(root)
            .with_mode(WorkspaceMode::Autonomous)
            .with_escape_policy(EscapePolicy::Allow);

        Self::new(
            "autonomous",
            "Minimal restrictions - agent can access most paths",
            config,
        )
    }

    /// Create a "ci" profile - optimized for CI/CD environments.
    ///
    /// - Guided mode
    /// - Allow common CI paths
    /// - Deny escape by default (fail fast)
    #[must_use]
    pub fn ci(root: impl Into<PathBuf>) -> Self {
        let config = WorkspaceConfig::new(root)
            .with_mode(WorkspaceMode::Guided)
            .with_escape_policy(EscapePolicy::Deny)
            // CI-specific paths
            .allow_read("/tmp")
            .allow_write("/tmp");

        Self::new(
            "ci",
            "CI/CD optimized - fail fast on unexpected operations",
            config,
        )
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Get a profile by name.
#[must_use]
pub fn get_profile(name: &str, root: impl Into<PathBuf>) -> Option<WorkspaceProfile> {
    let root = root.into();
    match name {
        "safe" => Some(WorkspaceProfile::safe(root)),
        "power_user" => Some(WorkspaceProfile::power_user(root)),
        "autonomous" => Some(WorkspaceProfile::autonomous(root)),
        "ci" => Some(WorkspaceProfile::ci(root)),
        _ => None,
    }
}

/// List available profile names.
#[must_use]
pub fn available_profiles() -> Vec<&'static str> {
    vec!["safe", "power_user", "autonomous", "ci"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_profile() {
        let profile = WorkspaceProfile::safe("/project");
        assert_eq!(profile.name, "safe");
        assert_eq!(profile.config.mode, WorkspaceMode::Safe);
        assert_eq!(profile.config.escape_policy, EscapePolicy::Ask);
    }

    #[test]
    fn test_power_user_profile() {
        let profile = WorkspaceProfile::power_user("/project");
        assert_eq!(profile.name, "power_user");
        assert_eq!(profile.config.mode, WorkspaceMode::Guided);
    }

    #[test]
    fn test_autonomous_profile() {
        let profile = WorkspaceProfile::autonomous("/project");
        assert_eq!(profile.name, "autonomous");
        assert_eq!(profile.config.mode, WorkspaceMode::Autonomous);
        assert_eq!(profile.config.escape_policy, EscapePolicy::Allow);
    }

    #[test]
    fn test_ci_profile() {
        let profile = WorkspaceProfile::ci("/project");
        assert_eq!(profile.name, "ci");
        assert_eq!(profile.config.escape_policy, EscapePolicy::Deny);
    }

    #[test]
    fn test_get_profile() {
        assert!(get_profile("safe", "/project").is_some());
        assert!(get_profile("unknown", "/project").is_none());
    }

    #[test]
    fn test_available_profiles() {
        let profiles = available_profiles();
        assert!(profiles.contains(&"safe"));
        assert!(profiles.contains(&"power_user"));
        assert!(profiles.contains(&"autonomous"));
        assert!(profiles.contains(&"ci"));
    }
}
