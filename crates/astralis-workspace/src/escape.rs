//! Escape request handling.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::boundaries::PathCheck;

/// A request to escape the workspace boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscapeRequest {
    /// Unique request ID.
    pub request_id: Uuid,
    /// Path being accessed.
    pub path: PathBuf,
    /// Operation being performed.
    pub operation: EscapeOperation,
    /// Reason for the request.
    pub reason: String,
    /// When the request was created.
    pub created_at: DateTime<Utc>,
    /// Tool that initiated the request (if applicable).
    #[serde(default)]
    pub tool_name: Option<String>,
    /// Server that initiated the request (if applicable).
    #[serde(default)]
    pub server_name: Option<String>,
}

impl EscapeRequest {
    /// Create a new escape request.
    #[must_use]
    pub fn new(
        path: impl Into<PathBuf>,
        operation: EscapeOperation,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            path: path.into(),
            operation,
            reason: reason.into(),
            created_at: Utc::now(),
            tool_name: None,
            server_name: None,
        }
    }

    /// Set the tool name.
    #[must_use]
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool_name = Some(tool.into());
        self
    }

    /// Set the server name.
    #[must_use]
    pub fn with_server(mut self, server: impl Into<String>) -> Self {
        self.server_name = Some(server.into());
        self
    }
}

/// Operation being performed outside the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscapeOperation {
    /// Reading a file.
    Read,
    /// Writing to a file.
    Write,
    /// Creating a file or directory.
    Create,
    /// Deleting a file or directory.
    Delete,
    /// Executing a file.
    Execute,
    /// Listing a directory.
    List,
}

impl std::fmt::Display for EscapeOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read => write!(f, "read"),
            Self::Write => write!(f, "write"),
            Self::Create => write!(f, "create"),
            Self::Delete => write!(f, "delete"),
            Self::Execute => write!(f, "execute"),
            Self::List => write!(f, "list"),
        }
    }
}

/// Decision on an escape request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscapeDecision {
    /// Allow this one time.
    AllowOnce,
    /// Allow for the current session.
    AllowSession,
    /// Allow always (remember this path).
    AllowAlways,
    /// Deny the request.
    Deny,
}

impl EscapeDecision {
    /// Check if this is an allow decision.
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        !matches!(self, Self::Deny)
    }

    /// Check if this should be remembered.
    #[must_use]
    pub fn should_remember(&self) -> bool {
        matches!(self, Self::AllowAlways)
    }
}

/// Serializable state for `EscapeHandler` (for persistence).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EscapeState {
    /// Paths that have been permanently remembered (`AllowAlways` decisions).
    pub remembered_paths: Vec<PathBuf>,
}

/// Escape request handler.
#[derive(Debug, Clone)]
pub struct EscapeHandler {
    /// Remembered paths (`AllowAlways` decisions).
    remembered_paths: std::collections::HashSet<PathBuf>,
    /// Session-allowed paths.
    session_paths: std::collections::HashSet<PathBuf>,
}

impl EscapeHandler {
    /// Create a new escape handler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            remembered_paths: std::collections::HashSet::new(),
            session_paths: std::collections::HashSet::new(),
        }
    }

    /// Process an escape decision.
    ///
    /// Paths are canonicalized before storing so that comparisons are
    /// consistent regardless of how the path was originally specified.
    pub fn process_decision(&mut self, request: &EscapeRequest, decision: EscapeDecision) {
        let canonical =
            std::fs::canonicalize(&request.path).unwrap_or_else(|_| request.path.clone());
        match decision {
            EscapeDecision::AllowAlways => {
                self.remembered_paths.insert(canonical);
            },
            EscapeDecision::AllowSession => {
                self.session_paths.insert(canonical);
            },
            _ => {},
        }
    }

    /// Check if a path has been allowed.
    ///
    /// The path is canonicalized before checking to match the stored form.
    #[must_use]
    pub fn is_allowed(&self, path: &PathBuf) -> bool {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        self.remembered_paths.contains(&canonical) || self.session_paths.contains(&canonical)
    }

    /// Clear session-allowed paths.
    pub fn clear_session(&mut self) {
        self.session_paths.clear();
    }

    /// Clear all remembered paths.
    pub fn clear_all(&mut self) {
        self.remembered_paths.clear();
        self.session_paths.clear();
    }

    /// Export the current state for persistence.
    #[must_use]
    pub fn export_state(&self) -> EscapeState {
        EscapeState {
            remembered_paths: self.remembered_paths.iter().cloned().collect(),
        }
    }

    /// Restore state from a previously exported state.
    ///
    /// Only absolute paths that can be canonicalized (i.e., exist on disk)
    /// are restored. This prevents workspace boundary bypass via injected
    /// relative or non-existent paths in the persisted state.
    pub fn restore_state(&mut self, state: EscapeState) {
        for path in state.remembered_paths {
            if path.is_absolute()
                && let Ok(canonical) = std::fs::canonicalize(&path)
            {
                self.remembered_paths.insert(canonical);
            }
            // Skip relative or non-existent paths (stale or injected)
        }
    }
}

impl Default for EscapeHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of checking escape flow.
#[derive(Debug, Clone)]
pub enum EscapeFlow {
    /// Path is allowed (in workspace or auto-allowed).
    Allowed,
    /// Path is denied (never-allowed).
    Denied,
    /// Path needs approval.
    NeedsApproval(EscapeRequest),
}

impl EscapeFlow {
    /// Create from a path check result.
    #[must_use]
    pub fn from_check(
        check: PathCheck,
        path: PathBuf,
        operation: EscapeOperation,
        reason: impl Into<String>,
    ) -> Self {
        match check {
            PathCheck::Allowed | PathCheck::AutoAllowed => Self::Allowed,
            PathCheck::NeverAllowed => Self::Denied,
            PathCheck::RequiresApproval => {
                Self::NeedsApproval(EscapeRequest::new(path, operation, reason))
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_request() {
        let request =
            EscapeRequest::new("/etc/passwd", EscapeOperation::Read, "Need to read config")
                .with_tool("read_file");

        assert_eq!(request.operation, EscapeOperation::Read);
        assert_eq!(request.tool_name, Some("read_file".to_string()));
    }

    #[test]
    fn test_escape_decision() {
        assert!(EscapeDecision::AllowOnce.is_allowed());
        assert!(EscapeDecision::AllowSession.is_allowed());
        assert!(EscapeDecision::AllowAlways.is_allowed());
        assert!(!EscapeDecision::Deny.is_allowed());

        assert!(EscapeDecision::AllowAlways.should_remember());
        assert!(!EscapeDecision::AllowOnce.should_remember());
    }

    #[test]
    fn test_escape_handler() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let request = EscapeRequest::new(&path, EscapeOperation::Read, "test");

        let mut handler = EscapeHandler::new();
        assert!(!handler.is_allowed(&path));

        handler.process_decision(&request, EscapeDecision::AllowAlways);
        assert!(handler.is_allowed(&path));

        handler.clear_all();
        assert!(!handler.is_allowed(&path));
    }

    #[test]
    fn test_escape_handler_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let request = EscapeRequest::new(&path, EscapeOperation::Read, "test");

        let mut handler = EscapeHandler::new();
        handler.process_decision(&request, EscapeDecision::AllowSession);
        assert!(handler.is_allowed(&path));

        handler.clear_session();
        assert!(!handler.is_allowed(&path));
    }

    #[test]
    fn test_escape_state_export_restore() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let path1 = dir1.path().to_path_buf();
        let path2 = dir2.path().to_path_buf();

        let mut handler = EscapeHandler::new();
        let request1 = EscapeRequest::new(&path1, EscapeOperation::Read, "test");
        handler.process_decision(&request1, EscapeDecision::AllowAlways);
        let request2 = EscapeRequest::new(&path2, EscapeOperation::Write, "test");
        handler.process_decision(&request2, EscapeDecision::AllowAlways);

        let state = handler.export_state();
        assert_eq!(state.remembered_paths.len(), 2);

        // Verify serialization roundtrip
        let json = serde_json::to_string(&state).unwrap();
        let restored_state: EscapeState = serde_json::from_str(&json).unwrap();

        let mut new_handler = EscapeHandler::new();
        new_handler.restore_state(restored_state);
        assert!(new_handler.is_allowed(&path1));
        assert!(new_handler.is_allowed(&path2));
    }

    #[test]
    fn test_escape_state_default() {
        let state = EscapeState::default();
        assert!(state.remembered_paths.is_empty());
    }

    #[test]
    fn test_escape_state_restore_merges() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let path1 = dir1.path().to_path_buf();
        let path2 = dir2.path().to_path_buf();

        let mut handler = EscapeHandler::new();
        let request1 = EscapeRequest::new(&path1, EscapeOperation::Read, "test");
        handler.process_decision(&request1, EscapeDecision::AllowAlways);

        // Restore additional paths — should merge, not replace
        let state = EscapeState {
            remembered_paths: vec![path2.clone()],
        };
        handler.restore_state(state);

        assert!(handler.is_allowed(&path1));
        assert!(handler.is_allowed(&path2));
    }

    #[test]
    fn test_restore_state_rejects_relative_paths() {
        let mut handler = EscapeHandler::new();
        let state = EscapeState {
            remembered_paths: vec![
                PathBuf::from("relative/path"),
                PathBuf::from("../escape/attempt"),
            ],
        };
        handler.restore_state(state);
        // Relative paths should be rejected
        assert!(handler.export_state().remembered_paths.is_empty());
    }

    #[test]
    fn test_restore_state_rejects_nonexistent_paths() {
        let mut handler = EscapeHandler::new();
        let state = EscapeState {
            remembered_paths: vec![PathBuf::from(
                "/nonexistent/path/that/does/not/exist/at/all",
            )],
        };
        handler.restore_state(state);
        // Non-existent paths should be rejected (canonicalize fails)
        assert!(handler.export_state().remembered_paths.is_empty());
    }
}
