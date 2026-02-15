//! Workspace configuration types.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Operating mode for the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    /// Always ask before operations outside workspace.
    #[default]
    Safe,
    /// Smart defaults with selective approval.
    Guided,
    /// No restrictions (agent machine mode).
    Autonomous,
}

/// Policy for handling escape requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscapePolicy {
    /// Always ask the user.
    #[default]
    Ask,
    /// Always deny escape requests.
    Deny,
    /// Always allow escape requests.
    Allow,
}

/// Paths that are automatically allowed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutoAllowPaths {
    /// Paths that are always allowed for reading.
    #[serde(default)]
    pub read: Vec<PathBuf>,
    /// Paths that are always allowed for writing.
    #[serde(default)]
    pub write: Vec<PathBuf>,
    /// Glob patterns for auto-allowed paths.
    #[serde(default)]
    pub patterns: Vec<String>,
}

/// Workspace configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Root directory of the workspace.
    pub root: PathBuf,
    /// Operating mode.
    #[serde(default)]
    pub mode: WorkspaceMode,
    /// Policy for escape requests.
    #[serde(default)]
    pub escape_policy: EscapePolicy,
    /// Paths that are automatically allowed.
    #[serde(default)]
    pub auto_allow: AutoAllowPaths,
    /// Paths that are never allowed (even with approval).
    #[serde(default)]
    pub never_allow: Vec<PathBuf>,
    /// Whether to allow creating files outside workspace.
    #[serde(default)]
    pub allow_create_outside: bool,
    /// Whether to allow deleting files outside workspace.
    #[serde(default)]
    pub allow_delete_outside: bool,
}

impl WorkspaceConfig {
    /// Create a new workspace configuration.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            mode: WorkspaceMode::Safe,
            escape_policy: EscapePolicy::Ask,
            auto_allow: AutoAllowPaths::default(),
            never_allow: vec![
                PathBuf::from("/etc"),
                PathBuf::from("/var"),
                PathBuf::from("/usr"),
                PathBuf::from("/bin"),
                PathBuf::from("/sbin"),
                PathBuf::from("/boot"),
                PathBuf::from("/root"),
            ],
            allow_create_outside: false,
            allow_delete_outside: false,
        }
    }

    /// Set the operating mode.
    #[must_use]
    pub fn with_mode(mut self, mode: WorkspaceMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set the escape policy.
    #[must_use]
    pub fn with_escape_policy(mut self, policy: EscapePolicy) -> Self {
        self.escape_policy = policy;
        self
    }

    /// Add an auto-allowed read path.
    #[must_use]
    pub fn allow_read(mut self, path: impl Into<PathBuf>) -> Self {
        self.auto_allow.read.push(path.into());
        self
    }

    /// Add an auto-allowed write path.
    #[must_use]
    pub fn allow_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.auto_allow.write.push(path.into());
        self
    }

    /// Add a never-allowed path.
    #[must_use]
    pub fn never_allow(mut self, path: impl Into<PathBuf>) -> Self {
        self.never_allow.push(path.into());
        self
    }

    /// Check if a path is in the workspace.
    #[must_use]
    pub fn is_in_workspace(&self, path: &std::path::Path) -> bool {
        path.starts_with(&self.root)
    }
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self::new(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_config_creation() {
        let config = WorkspaceConfig::new("/home/user/project");
        assert_eq!(config.root, PathBuf::from("/home/user/project"));
        assert_eq!(config.mode, WorkspaceMode::Safe);
    }

    #[test]
    fn test_workspace_mode() {
        let config = WorkspaceConfig::new("/test").with_mode(WorkspaceMode::Autonomous);
        assert_eq!(config.mode, WorkspaceMode::Autonomous);
    }

    #[test]
    fn test_is_in_workspace() {
        let config = WorkspaceConfig::new("/home/user/project");
        assert!(config.is_in_workspace(std::path::Path::new("/home/user/project/src/main.rs")));
        assert!(!config.is_in_workspace(std::path::Path::new("/home/user/other")));
    }
}
