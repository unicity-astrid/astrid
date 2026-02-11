//! Workspace boundary checking.

use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use crate::config::{EscapePolicy, WorkspaceConfig, WorkspaceMode};

/// Result of checking a path against workspace boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathCheck {
    /// Path is within the workspace, allowed.
    Allowed,
    /// Path is auto-allowed (outside workspace but configured).
    AutoAllowed,
    /// Path is never allowed (protected system path).
    NeverAllowed,
    /// Path requires user approval.
    RequiresApproval,
}

impl PathCheck {
    /// Check if the path is allowed (directly or auto).
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed | Self::AutoAllowed)
    }

    /// Check if the path requires approval.
    #[must_use]
    pub fn needs_approval(&self) -> bool {
        matches!(self, Self::RequiresApproval)
    }

    /// Check if the path is never allowed.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::NeverAllowed)
    }
}

/// Workspace boundary checker.
///
/// Pre-compiles glob patterns for efficient matching.
#[derive(Debug)]
pub struct WorkspaceBoundary {
    config: WorkspaceConfig,
    /// Pre-compiled glob matchers for auto-allow patterns.
    compiled_matchers: Vec<GlobMatcher>,
}

impl Clone for WorkspaceBoundary {
    fn clone(&self) -> Self {
        // Re-compile matchers when cloning
        Self::new(self.config.clone())
    }
}

impl WorkspaceBoundary {
    /// Create a new workspace boundary checker.
    ///
    /// Pre-compiles all glob patterns in the configuration.
    #[must_use]
    pub fn new(config: WorkspaceConfig) -> Self {
        let compiled_matchers = config
            .auto_allow
            .patterns
            .iter()
            .filter_map(|pattern| match Glob::new(pattern) {
                Ok(glob) => Some(glob.compile_matcher()),
                Err(e) => {
                    warn!(pattern = %pattern, error = %e, "Failed to compile glob pattern");
                    None
                },
            })
            .collect();

        Self {
            config,
            compiled_matchers,
        }
    }

    /// Get the workspace configuration.
    #[must_use]
    pub fn config(&self) -> &WorkspaceConfig {
        &self.config
    }

    /// Get the workspace root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.config.root
    }

    /// Check if a path is within the workspace.
    #[must_use]
    pub fn is_in_workspace(&self, path: &Path) -> bool {
        let expanded = self.expand_path(path);
        expanded.starts_with(&self.config.root)
    }

    /// Check if a path is auto-allowed.
    #[must_use]
    pub fn is_auto_allowed(&self, path: &Path) -> bool {
        let expanded = self.expand_path(path);

        // Check read paths
        for allowed in &self.config.auto_allow.read {
            if expanded.starts_with(allowed) {
                return true;
            }
        }

        // Check write paths
        for allowed in &self.config.auto_allow.write {
            if expanded.starts_with(allowed) {
                return true;
            }
        }

        // Check pre-compiled glob patterns
        for matcher in &self.compiled_matchers {
            if matcher.is_match(&expanded) {
                return true;
            }
        }

        false
    }

    /// Check if a path is never allowed.
    #[must_use]
    pub fn is_never_allowed(&self, path: &Path) -> bool {
        let expanded = self.expand_path(path);

        for blocked in &self.config.never_allow {
            // Canonicalize the blocked path too (handles symlinks like /etc -> /private/etc on macOS)
            let blocked_expanded = blocked.canonicalize().unwrap_or_else(|_| blocked.clone());
            if expanded.starts_with(&blocked_expanded) {
                return true;
            }
            // Also check without canonicalization for non-existent paths
            if expanded.starts_with(blocked) {
                return true;
            }
        }

        false
    }

    /// Check a path against the workspace boundaries.
    #[must_use]
    pub fn check(&self, path: &Path) -> PathCheck {
        let expanded = self.expand_path(path);

        debug!(
            path = %path.display(),
            expanded = %expanded.display(),
            "Checking path against workspace"
        );

        // Check never-allowed first
        if self.is_never_allowed(&expanded) {
            return PathCheck::NeverAllowed;
        }

        // Check if in workspace
        if self.is_in_workspace(&expanded) {
            return PathCheck::Allowed;
        }

        // Check auto-allowed
        if self.is_auto_allowed(&expanded) {
            return PathCheck::AutoAllowed;
        }

        // Check mode
        match self.config.mode {
            WorkspaceMode::Autonomous => PathCheck::Allowed,
            WorkspaceMode::Guided | WorkspaceMode::Safe => match self.config.escape_policy {
                EscapePolicy::Allow => PathCheck::AutoAllowed,
                EscapePolicy::Deny => PathCheck::NeverAllowed,
                EscapePolicy::Ask => PathCheck::RequiresApproval,
            },
        }
    }

    /// Expand a path to its canonical form.
    ///
    /// This resolves `.`, `..`, and symlinks if the path exists.
    #[must_use]
    pub fn expand_path(&self, path: &Path) -> PathBuf {
        // Try to canonicalize, fall back to the original path
        path.canonicalize().unwrap_or_else(|_| {
            // If the path doesn't exist, try to normalize it manually
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.config.root.join(path)
            }
        })
    }

    /// Check multiple paths and return the most restrictive result.
    #[must_use]
    pub fn check_all(&self, paths: &[&Path]) -> PathCheck {
        let mut result = PathCheck::Allowed;

        for path in paths {
            let check = self.check(path);
            match check {
                PathCheck::NeverAllowed => return PathCheck::NeverAllowed,
                PathCheck::RequiresApproval => result = PathCheck::RequiresApproval,
                PathCheck::AutoAllowed if result == PathCheck::Allowed => {
                    result = PathCheck::AutoAllowed;
                },
                _ => {},
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_path_check_helpers() {
        assert!(PathCheck::Allowed.is_allowed());
        assert!(PathCheck::AutoAllowed.is_allowed());
        assert!(!PathCheck::NeverAllowed.is_allowed());
        assert!(!PathCheck::RequiresApproval.is_allowed());

        assert!(PathCheck::RequiresApproval.needs_approval());
        assert!(!PathCheck::Allowed.needs_approval());

        assert!(PathCheck::NeverAllowed.is_blocked());
        assert!(!PathCheck::Allowed.is_blocked());
    }

    #[test]
    fn test_workspace_boundary_in_workspace() {
        let temp_dir = TempDir::new().unwrap();
        let config = WorkspaceConfig::new(temp_dir.path());
        let boundary = WorkspaceBoundary::new(config);

        let in_workspace = temp_dir.path().join("src/main.rs");
        assert!(boundary.is_in_workspace(&in_workspace));

        let outside = PathBuf::from("/tmp/other");
        assert!(!boundary.is_in_workspace(&outside));
    }

    #[test]
    fn test_workspace_boundary_never_allowed() {
        let config = WorkspaceConfig::new("/home/user/project").never_allow("/etc");
        let boundary = WorkspaceBoundary::new(config);

        assert!(boundary.is_never_allowed(Path::new("/etc/passwd")));
        assert_eq!(
            boundary.check(Path::new("/etc/passwd")),
            PathCheck::NeverAllowed
        );
    }

    #[test]
    fn test_workspace_boundary_auto_allowed() {
        let config = WorkspaceConfig::new("/home/user/project").allow_read("/usr/share/doc");
        let boundary = WorkspaceBoundary::new(config);

        assert!(boundary.is_auto_allowed(Path::new("/usr/share/doc/readme.txt")));
    }

    #[test]
    fn test_workspace_boundary_autonomous_mode() {
        let config =
            WorkspaceConfig::new("/home/user/project").with_mode(WorkspaceMode::Autonomous);
        let boundary = WorkspaceBoundary::new(config);

        // In autonomous mode, everything except never-allowed is allowed
        assert_eq!(
            boundary.check(Path::new("/tmp/random/file")),
            PathCheck::Allowed
        );
    }
}
