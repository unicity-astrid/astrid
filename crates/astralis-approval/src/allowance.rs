//! Allowance types and store for pre-approved action patterns.
//!
//! An [`Allowance`] grants pre-approved access for actions matching a specific
//! pattern. Created when users select "Allow Session" or "Create Allowance"
//! during approval flows.
//!
//! The [`AllowanceStore`] holds active allowances in memory, supporting
//! pattern-based matching, use tracking, expiration cleanup, and session clearing.

use astralis_core::error::{SecurityError, SecurityResult};
use astralis_core::types::{Permission, Timestamp};
use astralis_crypto::Signature;
use globset::Glob;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use uuid::Uuid;

use crate::action::SensitiveAction;

/// Unique identifier for an allowance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AllowanceId(pub Uuid);

impl AllowanceId {
    /// Create a new random allowance ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AllowanceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AllowanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "allowance:{}", self.0)
    }
}

/// Pattern describing what actions an allowance covers.
///
/// Each pattern variant matches a specific category of [`SensitiveAction`].
/// Use [`AllowancePattern::matches`] to check if an action is covered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AllowancePattern {
    /// Match a specific tool on a specific server.
    ExactTool {
        /// MCP server name.
        server: String,
        /// Tool name.
        tool: String,
    },

    /// Match all tools on a server.
    ServerTools {
        /// MCP server name.
        server: String,
    },

    /// Match file access by glob pattern.
    FilePattern {
        /// Glob pattern for file paths.
        pattern: String,
        /// Required permission.
        permission: Permission,
    },

    /// Match network access to a host.
    NetworkHost {
        /// Target hostname.
        host: String,
        /// Allowed ports (None = all ports).
        ports: Option<Vec<u16>>,
    },

    /// Match command execution by command name or glob pattern.
    CommandPattern {
        /// Command name or glob pattern.
        command: String,
    },

    /// Match actions scoped to a workspace directory.
    ///
    /// Workspace-relative allowances persist beyond session end but are scoped
    /// to the workspace root. They match the same action types as their base
    /// pattern (e.g., `ExactTool`, `FilePattern`) but only when the runtime
    /// is operating within the specified workspace.
    WorkspaceRelative {
        /// The base pattern to match against (same semantics as other variants).
        pattern: String,
        /// Permission required.
        permission: Permission,
    },

    /// Custom pattern string for extensibility.
    Custom {
        /// Pattern string (interpretation is context-dependent).
        pattern: String,
    },

    /// Match a specific plugin capability.
    ///
    /// For `PluginExecution`, matches on the `capability` field directly.
    /// For `PluginHttpRequest`, matches on `"http_request"`.
    /// For `PluginFileAccess`, matches on the derived capability name
    /// (`"file_read"`, `"file_write"`, `"file_delete"`).
    PluginCapability {
        /// Plugin identifier.
        plugin_id: String,
        /// Capability name to match.
        capability: String,
    },

    /// Match any action from a specific plugin (wildcard).
    PluginWildcard {
        /// Plugin identifier.
        plugin_id: String,
    },
}

impl AllowancePattern {
    /// Check if this pattern matches a sensitive action.
    ///
    /// Matching rules:
    /// - `ExactTool` matches `McpToolCall` with the same server and tool.
    /// - `ServerTools` matches any `McpToolCall` on that server.
    /// - `FilePattern` matches `FileRead` (Read permission), `FileDelete` (Delete permission),
    ///   or `FileWriteOutsideSandbox` (Write permission) when the path matches the glob.
    /// - `NetworkHost` matches `NetworkRequest` when host matches and port is allowed.
    /// - `WorkspaceRelative` variants additionally validate that the action's path
    ///   starts with `workspace_root` (if provided) before matching the pattern.
    /// - `Custom` never matches (extensibility point for future use).
    ///
    /// # Arguments
    ///
    /// * `workspace_root` — The current workspace root. For `WorkspaceRelative` patterns,
    ///   the action path must fall under this root for the match to succeed.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn matches(&self, action: &SensitiveAction, workspace_root: Option<&Path>) -> bool {
        match (self, action) {
            // ExactTool: exact server + tool match
            (
                Self::ExactTool { server, tool },
                SensitiveAction::McpToolCall {
                    server: action_server,
                    tool: action_tool,
                },
            ) => server == action_server && tool == action_tool,

            // ServerTools: any tool on the server
            (
                Self::ServerTools { server },
                SensitiveAction::McpToolCall {
                    server: action_server,
                    ..
                },
            ) => server == action_server,

            // FilePattern matches file actions (no workspace check needed)
            (
                Self::FilePattern {
                    pattern,
                    permission: Permission::Delete,
                },
                SensitiveAction::FileDelete { path },
            )
            | (
                Self::FilePattern {
                    pattern,
                    permission: Permission::Write,
                },
                SensitiveAction::FileWriteOutsideSandbox { path },
            )
            | (
                Self::FilePattern {
                    pattern,
                    permission: Permission::Read,
                },
                SensitiveAction::FileRead { path },
            ) => matches_file_glob(pattern, path),

            // WorkspaceRelative file patterns: path must be under workspace_root
            (
                Self::WorkspaceRelative {
                    pattern,
                    permission: Permission::Delete,
                },
                SensitiveAction::FileDelete { path },
            )
            | (
                Self::WorkspaceRelative {
                    pattern,
                    permission: Permission::Write,
                },
                SensitiveAction::FileWriteOutsideSandbox { path },
            )
            | (
                Self::WorkspaceRelative {
                    pattern,
                    permission: Permission::Read,
                },
                SensitiveAction::FileRead { path },
            ) => path_in_workspace(path, workspace_root) && matches_file_glob(pattern, path),

            // NetworkHost matches NetworkRequest
            (
                Self::NetworkHost { host, ports },
                SensitiveAction::NetworkRequest {
                    host: action_host,
                    port: action_port,
                },
            ) => {
                host == action_host
                    && ports
                        .as_ref()
                        .is_none_or(|allowed| allowed.contains(action_port))
            },

            // WorkspaceRelative with Invoke permission matches MCP tool calls
            (
                Self::WorkspaceRelative {
                    pattern,
                    permission: Permission::Invoke,
                },
                SensitiveAction::McpToolCall { server, tool },
            ) => {
                // For non-file actions, workspace_root must be provided to confirm
                // we're in a workspace context
                workspace_root.is_some() && {
                    let resource = format!("{server}/{tool}");
                    matches_file_glob(pattern, &resource)
                }
            },

            // WorkspaceRelative with Execute permission matches ExecuteCommand
            (
                Self::WorkspaceRelative {
                    pattern,
                    permission: Permission::Execute,
                },
                SensitiveAction::ExecuteCommand { command, .. },
            ) => workspace_root.is_some() && matches_file_glob(pattern, command),

            // CommandPattern matches ExecuteCommand (no workspace check)
            (
                Self::CommandPattern { command: pattern },
                SensitiveAction::ExecuteCommand { command, .. },
            ) => matches_file_glob(pattern, command),

            // PluginCapability: match plugin actions with same plugin_id + derived capability
            (
                Self::PluginCapability {
                    plugin_id,
                    capability,
                },
                SensitiveAction::PluginExecution {
                    plugin_id: action_pid,
                    capability: action_cap,
                },
            ) => plugin_id == action_pid && capability == action_cap,

            (
                Self::PluginCapability {
                    plugin_id,
                    capability,
                },
                SensitiveAction::PluginHttpRequest {
                    plugin_id: action_pid,
                    ..
                },
            ) => plugin_id == action_pid && capability == "http_request",

            (
                Self::PluginCapability {
                    plugin_id,
                    capability,
                },
                SensitiveAction::PluginFileAccess {
                    plugin_id: action_pid,
                    mode,
                    ..
                },
            ) => {
                plugin_id == action_pid
                    && capability
                        == match mode {
                            Permission::Read => "file_read",
                            Permission::Write => "file_write",
                            Permission::Delete => "file_delete",
                            _ => return false,
                        }
            },

            // PluginWildcard: match any plugin action from the same plugin_id
            (
                Self::PluginWildcard { plugin_id },
                SensitiveAction::PluginExecution {
                    plugin_id: action_pid,
                    ..
                }
                | SensitiveAction::PluginHttpRequest {
                    plugin_id: action_pid,
                    ..
                }
                | SensitiveAction::PluginFileAccess {
                    plugin_id: action_pid,
                    ..
                },
            ) => plugin_id == action_pid,

            // Custom never matches (future extensibility), and all other combinations don't match
            _ => false,
        }
    }
}

/// Check if a path falls under the given workspace root.
///
/// If `workspace_root` is `None`, the check passes (no workspace constraint).
/// This is used for `WorkspaceRelative` allowance patterns to ensure that
/// an allowance created in `/project-a` cannot match actions in `/project-b`.
fn path_in_workspace(path: &str, workspace_root: Option<&Path>) -> bool {
    match workspace_root {
        None => true,
        Some(root) => {
            let path = Path::new(path);
            path.starts_with(root)
        },
    }
}

/// Check if a file path matches a glob pattern, with path traversal protection.
fn matches_file_glob(pattern: &str, path: &str) -> bool {
    // Reject path traversal using std::path::Path::components() for robustness
    if Path::new(path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }

    Glob::new(pattern)
        .ok()
        .is_some_and(|glob| glob.compile_matcher().is_match(path))
}

impl fmt::Display for AllowancePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExactTool { server, tool } => write!(f, "mcp://{server}/{tool}"),
            Self::ServerTools { server } => write!(f, "mcp://{server}/*"),
            Self::FilePattern {
                pattern,
                permission,
            } => write!(f, "file:{pattern} ({permission})"),
            Self::NetworkHost { host, ports } => {
                if let Some(ports) = ports {
                    let port_list: Vec<_> = ports.iter().map(ToString::to_string).collect();
                    write!(f, "net:{host}:[{}]", port_list.join(","))
                } else {
                    write!(f, "net:{host}:*")
                }
            },
            Self::CommandPattern { command } => write!(f, "cmd:{command}"),
            Self::WorkspaceRelative {
                pattern,
                permission,
            } => write!(f, "workspace:{pattern} ({permission})"),
            Self::Custom { pattern } => write!(f, "custom:{pattern}"),
            Self::PluginCapability {
                plugin_id,
                capability,
            } => write!(f, "plugin://{plugin_id}:{capability}"),
            Self::PluginWildcard { plugin_id } => write!(f, "plugin://{plugin_id}:*"),
        }
    }
}

/// An allowance granting pre-approved access for actions matching a pattern.
///
/// Allowances are created during approval flows:
/// - **Session allowances** (`session_only: true`): Cleared when the session ends.
/// - **Persistent allowances**: Survive across sessions (backed by capability tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Allowance {
    /// Unique allowance identifier.
    pub id: AllowanceId,
    /// Pattern describing what actions this allowance covers.
    pub action_pattern: AllowancePattern,
    /// When the allowance was created.
    pub created_at: Timestamp,
    /// When the allowance expires (None = no expiration within scope).
    pub expires_at: Option<Timestamp>,
    /// Maximum number of uses (None = unlimited).
    pub max_uses: Option<u32>,
    /// Remaining uses (None = unlimited, decremented on each use).
    pub uses_remaining: Option<u32>,
    /// Whether this allowance is scoped to the current session only.
    pub session_only: bool,
    /// Workspace root this allowance is scoped to (None = not workspace-scoped).
    pub workspace_root: Option<PathBuf>,
    /// Cryptographic signature proving this allowance was legitimately created.
    pub signature: Signature,
}

impl Allowance {
    /// Check if the allowance has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.expires_at.as_ref().is_some_and(Timestamp::is_past)
    }

    /// Check if the allowance has uses remaining.
    #[must_use]
    pub fn has_uses_remaining(&self) -> bool {
        self.uses_remaining.is_none_or(|r| r > 0)
    }

    /// Check if the allowance is still valid (not expired, has uses).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.is_expired() && self.has_uses_remaining()
    }
}

// ---------------------------------------------------------------------------
// AllowanceStore
// ---------------------------------------------------------------------------

/// In-memory store for active allowances.
///
/// Thread-safe via internal [`RwLock`]. Supports pattern-based matching,
/// use tracking, expiration cleanup, and session clearing.
///
/// # Example
///
/// ```
/// use astralis_approval::AllowanceStore;
///
/// let store = AllowanceStore::new();
/// assert_eq!(store.count(), 0);
/// ```
pub struct AllowanceStore {
    allowances: RwLock<HashMap<AllowanceId, Allowance>>,
}

impl AllowanceStore {
    /// Create a new empty allowance store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            allowances: RwLock::new(HashMap::new()),
        }
    }

    /// Add an allowance to the store.
    ///
    /// # Errors
    ///
    /// Returns a storage error if the internal lock is poisoned.
    pub fn add_allowance(&self, allowance: Allowance) -> SecurityResult<()> {
        let mut store = self
            .allowances
            .write()
            .map_err(|e| SecurityError::StorageError(e.to_string()))?;
        store.insert(allowance.id.clone(), allowance);
        Ok(())
    }

    /// Find the first valid allowance that matches an action.
    ///
    /// An allowance matches when:
    /// 1. Its pattern covers the action
    /// 2. It has not expired
    /// 3. It has uses remaining (if limited)
    /// 4. For workspace-scoped allowances (`workspace_root: Some(..)`),
    ///    the allowance's `workspace_root` must match the current `workspace_root`
    ///
    /// Returns a clone of the matching allowance, or `None`.
    #[must_use]
    pub fn find_matching(
        &self,
        action: &SensitiveAction,
        workspace_root: Option<&Path>,
    ) -> Option<Allowance> {
        let store = self.allowances.read().unwrap_or_else(|e| {
            tracing::warn!("AllowanceStore read lock poisoned, recovering");
            e.into_inner()
        });
        store
            .values()
            .find(|a| {
                if !a.is_valid() {
                    return false;
                }
                // Workspace-scoped allowances only match when the workspace root matches
                if let Some(allowance_ws) = &a.workspace_root
                    && workspace_root != Some(allowance_ws.as_path())
                {
                    return false;
                }
                a.action_pattern.matches(action, workspace_root)
            })
            .cloned()
    }

    /// Atomically find a matching allowance and consume one use.
    ///
    /// This combines [`find_matching`](Self::find_matching) and
    /// [`consume_use`](Self::consume_use) under a single write lock to prevent
    /// race conditions where two concurrent callers both find the same
    /// single-use allowance.
    ///
    /// Also cleans up expired allowances while the lock is held.
    ///
    /// Returns a clone of the matching allowance (before consumption), or `None`.
    #[must_use]
    pub fn find_matching_and_consume(
        &self,
        action: &SensitiveAction,
        workspace_root: Option<&Path>,
    ) -> Option<Allowance> {
        let mut store = self.allowances.write().unwrap_or_else(|e| {
            tracing::warn!("AllowanceStore lock poisoned, recovering");
            e.into_inner()
        });
        // Clean expired while we hold the lock
        store.retain(|_, a| !a.is_expired());
        let id = store
            .values()
            .find(|a| {
                a.is_valid()
                    && match &a.workspace_root {
                        Some(ws) => workspace_root == Some(ws.as_path()),
                        None => true,
                    }
                    && a.action_pattern.matches(action, workspace_root)
            })?
            .id
            .clone();
        let allowance = store.get(&id)?.clone();
        // Consume use atomically
        if let Some(remaining) = store.get_mut(&id).and_then(|a| a.uses_remaining.as_mut()) {
            *remaining = remaining.saturating_sub(1);
        }
        Some(allowance)
    }

    /// Consume one use of an allowance.
    ///
    /// For unlimited allowances (`uses_remaining: None`), this is a no-op.
    /// For limited allowances, decrements `uses_remaining` by 1.
    ///
    /// Returns `true` if the allowance still has uses remaining after consumption,
    /// `false` if this was the last use.
    ///
    /// # Errors
    ///
    /// Returns an error if the allowance is not found or the lock is poisoned.
    pub fn consume_use(&self, allowance_id: &AllowanceId) -> SecurityResult<bool> {
        let mut store = self
            .allowances
            .write()
            .map_err(|e| SecurityError::StorageError(e.to_string()))?;

        let allowance = store.get_mut(allowance_id).ok_or_else(|| {
            SecurityError::StorageError(format!("allowance not found: {allowance_id}"))
        })?;

        if let Some(remaining) = &mut allowance.uses_remaining {
            *remaining = remaining.saturating_sub(1);
            Ok(*remaining > 0)
        } else {
            // Unlimited — always has uses remaining
            Ok(true)
        }
    }

    /// Remove all expired allowances from the store.
    ///
    /// Returns the number of allowances removed.
    pub fn cleanup_expired(&self) -> usize {
        let Ok(mut store) = self.allowances.write() else {
            return 0;
        };
        let before = store.len();
        store.retain(|_, a| !a.is_expired());
        before - store.len()
    }

    /// Remove all session-only allowances from the store.
    ///
    /// Called when a session ends to clear temporary permissions.
    pub fn clear_session_allowances(&self) {
        if let Ok(mut store) = self.allowances.write() {
            store.retain(|_, a| !a.session_only);
        }
    }

    /// Get the number of allowances in the store.
    #[must_use]
    pub fn count(&self) -> usize {
        self.allowances.read().map(|s| s.len()).unwrap_or(0)
    }

    /// Export all session-scoped allowances for persistence.
    ///
    /// Returns a list of allowances that have `session_only: true`.
    /// These are the allowances that would be lost on restart without persistence.
    #[must_use]
    pub fn export_session_allowances(&self) -> Vec<Allowance> {
        self.allowances
            .read()
            .map(|store| {
                store
                    .values()
                    .filter(|a| a.session_only && a.is_valid())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Export all workspace-scoped allowances for persistence.
    ///
    /// Returns allowances that have `session_only: false` and a `workspace_root` set.
    /// These are the allowances that should be persisted in the workspace `state.db`.
    #[must_use]
    pub fn export_workspace_allowances(&self) -> Vec<Allowance> {
        self.allowances
            .read()
            .map(|store| {
                store
                    .values()
                    .filter(|a| !a.session_only && a.workspace_root.is_some() && a.is_valid())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Import allowances into the store, merging with existing ones.
    ///
    /// Used to restore session allowances from a persisted session.
    pub fn import_allowances(&self, allowances: Vec<Allowance>) {
        if let Ok(mut store) = self.allowances.write() {
            for allowance in allowances {
                if allowance.is_valid() {
                    store.insert(allowance.id.clone(), allowance);
                }
            }
        }
    }
}

impl Default for AllowanceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AllowanceStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.count();
        f.debug_struct("AllowanceStore")
            .field("count", &count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astralis_crypto::KeyPair;

    /// Create a test allowance with the given pattern.
    fn make_allowance(pattern: AllowancePattern, session_only: bool) -> Allowance {
        let keypair = KeyPair::generate();
        Allowance {
            id: AllowanceId::new(),
            action_pattern: pattern,
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            session_only,
            workspace_root: None,
            signature: keypair.sign(b"test-allowance"),
        }
    }

    /// Create a limited-use test allowance.
    fn make_limited_allowance(pattern: AllowancePattern, max_uses: u32) -> Allowance {
        let keypair = KeyPair::generate();
        Allowance {
            id: AllowanceId::new(),
            action_pattern: pattern,
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: Some(max_uses),
            uses_remaining: Some(max_uses),
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test-allowance"),
        }
    }

    // -----------------------------------------------------------------------
    // AllowanceId tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_allowance_id() {
        let id1 = AllowanceId::new();
        let id2 = AllowanceId::new();
        assert_ne!(id1, id2);
        assert!(id1.to_string().starts_with("allowance:"));
    }

    // -----------------------------------------------------------------------
    // AllowancePattern display tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_allowance_pattern_display() {
        let pattern = AllowancePattern::ExactTool {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        assert_eq!(pattern.to_string(), "mcp://filesystem/read_file");

        let pattern = AllowancePattern::ServerTools {
            server: "github".to_string(),
        };
        assert_eq!(pattern.to_string(), "mcp://github/*");

        let pattern = AllowancePattern::NetworkHost {
            host: "api.example.com".to_string(),
            ports: Some(vec![443, 8080]),
        };
        assert_eq!(pattern.to_string(), "net:api.example.com:[443,8080]");

        let pattern = AllowancePattern::NetworkHost {
            host: "api.example.com".to_string(),
            ports: None,
        };
        assert_eq!(pattern.to_string(), "net:api.example.com:*");
    }

    #[test]
    fn test_allowance_pattern_serialization() {
        let pattern = AllowancePattern::FilePattern {
            pattern: "/home/user/docs/*".to_string(),
            permission: Permission::Read,
        };
        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: AllowancePattern = serde_json::from_str(&json).unwrap();
        assert_eq!(pattern.to_string(), deserialized.to_string());
    }

    // -----------------------------------------------------------------------
    // AllowancePattern matching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_exact_tool_matches() {
        let pattern = AllowancePattern::ExactTool {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        let action = SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        assert!(pattern.matches(&action, None));
    }

    #[test]
    fn test_exact_tool_wrong_tool() {
        let pattern = AllowancePattern::ExactTool {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        let action = SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "write_file".to_string(),
        };
        assert!(!pattern.matches(&action, None));
    }

    #[test]
    fn test_exact_tool_wrong_action_type() {
        let pattern = AllowancePattern::ExactTool {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        let action = SensitiveAction::FileDelete {
            path: "/tmp/test".to_string(),
        };
        assert!(!pattern.matches(&action, None));
    }

    #[test]
    fn test_server_tools_matches_any_tool() {
        let pattern = AllowancePattern::ServerTools {
            server: "filesystem".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::McpToolCall {
                server: "filesystem".to_string(),
                tool: "read_file".to_string(),
            },
            None
        ));
        assert!(pattern.matches(
            &SensitiveAction::McpToolCall {
                server: "filesystem".to_string(),
                tool: "write_file".to_string(),
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::McpToolCall {
                server: "github".to_string(),
                tool: "create_issue".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_file_pattern_delete() {
        let pattern = AllowancePattern::FilePattern {
            pattern: "/tmp/**".to_string(),
            permission: Permission::Delete,
        };
        assert!(pattern.matches(
            &SensitiveAction::FileDelete {
                path: "/tmp/build/output.o".to_string(),
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::FileDelete {
                path: "/home/user/important.txt".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_file_pattern_write() {
        let pattern = AllowancePattern::FilePattern {
            pattern: "/home/user/docs/*".to_string(),
            permission: Permission::Write,
        };
        assert!(pattern.matches(
            &SensitiveAction::FileWriteOutsideSandbox {
                path: "/home/user/docs/report.txt".to_string(),
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::FileWriteOutsideSandbox {
                path: "/etc/passwd".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_file_pattern_permission_mismatch() {
        // Write pattern does not match FileDelete
        let pattern = AllowancePattern::FilePattern {
            pattern: "/tmp/**".to_string(),
            permission: Permission::Write,
        };
        assert!(!pattern.matches(
            &SensitiveAction::FileDelete {
                path: "/tmp/file.txt".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_file_pattern_rejects_path_traversal() {
        let pattern = AllowancePattern::FilePattern {
            pattern: "/home/user/**".to_string(),
            permission: Permission::Delete,
        };
        assert!(!pattern.matches(
            &SensitiveAction::FileDelete {
                path: "/home/user/../../etc/passwd".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_network_host_matches() {
        let pattern = AllowancePattern::NetworkHost {
            host: "api.example.com".to_string(),
            ports: None,
        };
        assert!(pattern.matches(
            &SensitiveAction::NetworkRequest {
                host: "api.example.com".to_string(),
                port: 443,
            },
            None
        ));
        assert!(pattern.matches(
            &SensitiveAction::NetworkRequest {
                host: "api.example.com".to_string(),
                port: 8080,
            },
            None
        ));
    }

    #[test]
    fn test_network_host_with_ports() {
        let pattern = AllowancePattern::NetworkHost {
            host: "api.example.com".to_string(),
            ports: Some(vec![443, 8443]),
        };
        assert!(pattern.matches(
            &SensitiveAction::NetworkRequest {
                host: "api.example.com".to_string(),
                port: 443,
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::NetworkRequest {
                host: "api.example.com".to_string(),
                port: 80,
            },
            None
        ));
    }

    #[test]
    fn test_network_host_wrong_host() {
        let pattern = AllowancePattern::NetworkHost {
            host: "api.example.com".to_string(),
            ports: None,
        };
        assert!(!pattern.matches(
            &SensitiveAction::NetworkRequest {
                host: "evil.com".to_string(),
                port: 443,
            },
            None
        ));
    }

    #[test]
    fn test_custom_never_matches() {
        let pattern = AllowancePattern::Custom {
            pattern: "anything".to_string(),
        };
        assert!(!pattern.matches(
            &SensitiveAction::FileDelete {
                path: "/tmp/file".to_string(),
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::McpToolCall {
                server: "anything".to_string(),
                tool: "anything".to_string(),
            },
            None
        ));
    }

    // -----------------------------------------------------------------------
    // Allowance validity tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_allowance_valid_no_limits() {
        let allowance = make_allowance(
            AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            true,
        );
        assert!(!allowance.is_expired());
        assert!(allowance.has_uses_remaining());
        assert!(allowance.is_valid());
    }

    #[test]
    fn test_allowance_expired() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            created_at: Timestamp::from_datetime(chrono::Utc::now() - chrono::Duration::hours(2)),
            expires_at: Some(Timestamp::from_datetime(
                chrono::Utc::now() - chrono::Duration::hours(1),
            )),
            max_uses: None,
            uses_remaining: None,
            session_only: false,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        assert!(allowance.is_expired());
        assert!(!allowance.is_valid());
    }

    #[test]
    fn test_allowance_uses_exhausted() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: Some(5),
            uses_remaining: Some(0),
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        assert!(!allowance.has_uses_remaining());
        assert!(!allowance.is_valid());
    }

    #[test]
    fn test_allowance_uses_remaining() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: Some(5),
            uses_remaining: Some(3),
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        assert!(allowance.has_uses_remaining());
        assert!(allowance.is_valid());
    }

    #[test]
    fn test_allowance_serialization_roundtrip() {
        let allowance = make_allowance(
            AllowancePattern::ExactTool {
                server: "test".to_string(),
                tool: "test_tool".to_string(),
            },
            true,
        );
        let json = serde_json::to_string(&allowance).unwrap();
        let deserialized: Allowance = serde_json::from_str(&json).unwrap();
        assert_eq!(allowance.id, deserialized.id);
        assert_eq!(allowance.session_only, deserialized.session_only);
    }

    // -----------------------------------------------------------------------
    // AllowanceStore tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_store_add_and_count() {
        let store = AllowanceStore::new();
        assert_eq!(store.count(), 0);

        let allowance = make_allowance(
            AllowancePattern::ServerTools {
                server: "fs".to_string(),
            },
            true,
        );
        store.add_allowance(allowance).unwrap();
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_store_find_matching() {
        let store = AllowanceStore::new();

        let allowance = make_allowance(
            AllowancePattern::ExactTool {
                server: "filesystem".to_string(),
                tool: "read_file".to_string(),
            },
            true,
        );
        let expected_id = allowance.id.clone();
        store.add_allowance(allowance).unwrap();

        let action = SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        let found = store.find_matching(&action, None);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, expected_id);
    }

    #[test]
    fn test_store_find_matching_no_match() {
        let store = AllowanceStore::new();

        let allowance = make_allowance(
            AllowancePattern::ExactTool {
                server: "filesystem".to_string(),
                tool: "read_file".to_string(),
            },
            true,
        );
        store.add_allowance(allowance).unwrap();

        let action = SensitiveAction::McpToolCall {
            server: "github".to_string(),
            tool: "create_issue".to_string(),
        };
        assert!(store.find_matching(&action, None).is_none());
    }

    #[test]
    fn test_store_find_matching_skips_expired() {
        let store = AllowanceStore::new();

        let keypair = KeyPair::generate();
        let expired = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "filesystem".to_string(),
            },
            created_at: Timestamp::from_datetime(chrono::Utc::now() - chrono::Duration::hours(2)),
            expires_at: Some(Timestamp::from_datetime(
                chrono::Utc::now() - chrono::Duration::hours(1),
            )),
            max_uses: None,
            uses_remaining: None,
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        store.add_allowance(expired).unwrap();

        let action = SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        assert!(store.find_matching(&action, None).is_none());
    }

    #[test]
    fn test_store_find_matching_skips_exhausted() {
        let store = AllowanceStore::new();

        let mut allowance = make_limited_allowance(
            AllowancePattern::ServerTools {
                server: "filesystem".to_string(),
            },
            1,
        );
        let id = allowance.id.clone();
        // Pre-exhaust the uses
        allowance.uses_remaining = Some(0);
        store.add_allowance(allowance).unwrap();

        let action = SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        assert!(store.find_matching(&action, None).is_none());

        // Verify it's still in the store (not removed, just skipped)
        assert_eq!(store.count(), 1);
        // But consume_use on it still works (it's found by ID)
        assert!(store.consume_use(&id).is_ok());
    }

    #[test]
    fn test_store_consume_use_limited() {
        let store = AllowanceStore::new();

        let allowance = make_limited_allowance(
            AllowancePattern::ServerTools {
                server: "fs".to_string(),
            },
            3,
        );
        let id = allowance.id.clone();
        store.add_allowance(allowance).unwrap();

        // 3 uses: consume down to 2, 1, 0
        assert_eq!(store.consume_use(&id).unwrap(), true); // 2 remaining
        assert_eq!(store.consume_use(&id).unwrap(), true); // 1 remaining
        assert_eq!(store.consume_use(&id).unwrap(), false); // 0 remaining (last use)

        // Saturates at 0
        assert_eq!(store.consume_use(&id).unwrap(), false);
    }

    #[test]
    fn test_store_consume_use_unlimited() {
        let store = AllowanceStore::new();

        let allowance = make_allowance(
            AllowancePattern::ServerTools {
                server: "fs".to_string(),
            },
            true,
        );
        let id = allowance.id.clone();
        store.add_allowance(allowance).unwrap();

        // Unlimited: always returns true
        assert_eq!(store.consume_use(&id).unwrap(), true);
        assert_eq!(store.consume_use(&id).unwrap(), true);
    }

    #[test]
    fn test_store_consume_use_not_found() {
        let store = AllowanceStore::new();
        let result = store.consume_use(&AllowanceId::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_store_cleanup_expired() {
        let store = AllowanceStore::new();

        let keypair = KeyPair::generate();

        // Add an expired allowance
        let expired = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "old".to_string(),
            },
            created_at: Timestamp::from_datetime(chrono::Utc::now() - chrono::Duration::hours(2)),
            expires_at: Some(Timestamp::from_datetime(
                chrono::Utc::now() - chrono::Duration::hours(1),
            )),
            max_uses: None,
            uses_remaining: None,
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"expired"),
        };
        store.add_allowance(expired).unwrap();

        // Add a valid allowance
        let valid = make_allowance(
            AllowancePattern::ServerTools {
                server: "current".to_string(),
            },
            true,
        );
        store.add_allowance(valid).unwrap();

        assert_eq!(store.count(), 2);
        let removed = store.cleanup_expired();
        assert_eq!(removed, 1);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_store_clear_session_allowances() {
        let store = AllowanceStore::new();

        // Session allowance
        let session = make_allowance(
            AllowancePattern::ServerTools {
                server: "session-server".to_string(),
            },
            true,
        );
        store.add_allowance(session).unwrap();

        // Non-session allowance
        let persistent = make_allowance(
            AllowancePattern::ServerTools {
                server: "persistent-server".to_string(),
            },
            false,
        );
        store.add_allowance(persistent).unwrap();

        assert_eq!(store.count(), 2);
        store.clear_session_allowances();
        assert_eq!(store.count(), 1);

        // The persistent one should still be matchable
        let action = SensitiveAction::McpToolCall {
            server: "persistent-server".to_string(),
            tool: "any_tool".to_string(),
        };
        assert!(store.find_matching(&action, None).is_some());

        // The session one should be gone
        let action = SensitiveAction::McpToolCall {
            server: "session-server".to_string(),
            tool: "any_tool".to_string(),
        };
        assert!(store.find_matching(&action, None).is_none());
    }

    #[test]
    fn test_store_default() {
        let store = AllowanceStore::default();
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_store_debug() {
        let store = AllowanceStore::new();
        let debug = format!("{store:?}");
        assert!(debug.contains("AllowanceStore"));
        assert!(debug.contains("count"));
    }

    // -----------------------------------------------------------------------
    // FileRead matching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_file_pattern_read() {
        let pattern = AllowancePattern::FilePattern {
            pattern: "/home/user/docs/**".to_string(),
            permission: Permission::Read,
        };
        assert!(pattern.matches(
            &SensitiveAction::FileRead {
                path: "/home/user/docs/report.txt".to_string(),
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::FileRead {
                path: "/etc/passwd".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_file_pattern_read_does_not_match_write() {
        let pattern = AllowancePattern::FilePattern {
            pattern: "/home/user/**".to_string(),
            permission: Permission::Read,
        };
        // Read pattern must NOT match FileWriteOutsideSandbox
        assert!(!pattern.matches(
            &SensitiveAction::FileWriteOutsideSandbox {
                path: "/home/user/file.txt".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_workspace_relative_read() {
        let pattern = AllowancePattern::WorkspaceRelative {
            pattern: "/project/src/**".to_string(),
            permission: Permission::Read,
        };
        assert!(pattern.matches(
            &SensitiveAction::FileRead {
                path: "/project/src/main.rs".to_string(),
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::FileRead {
                path: "/other/path/file.rs".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_file_pattern_read_rejects_path_traversal() {
        let pattern = AllowancePattern::FilePattern {
            pattern: "/home/user/**".to_string(),
            permission: Permission::Read,
        };
        assert!(!pattern.matches(
            &SensitiveAction::FileRead {
                path: "/home/user/../../etc/passwd".to_string(),
            },
            None
        ));
    }

    // -----------------------------------------------------------------------
    // CommandPattern matching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_command_pattern_exact_match() {
        let pattern = AllowancePattern::CommandPattern {
            command: "cargo".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::ExecuteCommand {
                command: "cargo".to_string(),
                args: vec!["build".to_string()],
            },
            None
        ));
    }

    #[test]
    fn test_command_pattern_glob_match() {
        let pattern = AllowancePattern::CommandPattern {
            command: "cargo*".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::ExecuteCommand {
                command: "cargo".to_string(),
                args: vec![],
            },
            None
        ));
    }

    #[test]
    fn test_command_pattern_no_match() {
        let pattern = AllowancePattern::CommandPattern {
            command: "cargo".to_string(),
        };
        assert!(!pattern.matches(
            &SensitiveAction::ExecuteCommand {
                command: "sudo".to_string(),
                args: vec![],
            },
            None
        ));
    }

    #[test]
    fn test_command_pattern_does_not_match_other_action_types() {
        let pattern = AllowancePattern::CommandPattern {
            command: "cargo".to_string(),
        };
        assert!(!pattern.matches(
            &SensitiveAction::FileRead {
                path: "cargo".to_string(),
            },
            None
        ));
        assert!(!pattern.matches(
            &SensitiveAction::McpToolCall {
                server: "cargo".to_string(),
                tool: "build".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_command_pattern_display() {
        let pattern = AllowancePattern::CommandPattern {
            command: "cargo".to_string(),
        };
        assert_eq!(pattern.to_string(), "cmd:cargo");
    }

    // -----------------------------------------------------------------------
    // PluginCapability matching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_plugin_capability_matches_execution() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "weather".to_string(),
            capability: "config_read".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginExecution {
                plugin_id: "weather".to_string(),
                capability: "config_read".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_wrong_plugin() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "weather".to_string(),
            capability: "config_read".to_string(),
        };
        assert!(!pattern.matches(
            &SensitiveAction::PluginExecution {
                plugin_id: "other".to_string(),
                capability: "config_read".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_wrong_capability() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "weather".to_string(),
            capability: "config_read".to_string(),
        };
        assert!(!pattern.matches(
            &SensitiveAction::PluginExecution {
                plugin_id: "weather".to_string(),
                capability: "config_write".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_matches_http_request() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "weather".to_string(),
            capability: "http_request".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginHttpRequest {
                plugin_id: "weather".to_string(),
                url: "https://api.weather.com".to_string(),
                method: "GET".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_wrong_cap_for_http() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "weather".to_string(),
            capability: "file_read".to_string(),
        };
        assert!(!pattern.matches(
            &SensitiveAction::PluginHttpRequest {
                plugin_id: "weather".to_string(),
                url: "https://api.weather.com".to_string(),
                method: "GET".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_matches_file_read() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "cache".to_string(),
            capability: "file_read".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginFileAccess {
                plugin_id: "cache".to_string(),
                path: "/tmp/data".to_string(),
                mode: Permission::Read,
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_matches_file_write() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "cache".to_string(),
            capability: "file_write".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginFileAccess {
                plugin_id: "cache".to_string(),
                path: "/tmp/data".to_string(),
                mode: Permission::Write,
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_matches_file_delete() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "cache".to_string(),
            capability: "file_delete".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginFileAccess {
                plugin_id: "cache".to_string(),
                path: "/tmp/data".to_string(),
                mode: Permission::Delete,
            },
            None
        ));
    }

    #[test]
    fn test_plugin_capability_file_mode_mismatch() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "cache".to_string(),
            capability: "file_read".to_string(),
        };
        // file_read pattern should NOT match Write mode
        assert!(!pattern.matches(
            &SensitiveAction::PluginFileAccess {
                plugin_id: "cache".to_string(),
                path: "/tmp/data".to_string(),
                mode: Permission::Write,
            },
            None
        ));
    }

    // -----------------------------------------------------------------------
    // PluginWildcard matching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_plugin_wildcard_matches_execution() {
        let pattern = AllowancePattern::PluginWildcard {
            plugin_id: "weather".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginExecution {
                plugin_id: "weather".to_string(),
                capability: "anything".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_wildcard_matches_http() {
        let pattern = AllowancePattern::PluginWildcard {
            plugin_id: "weather".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginHttpRequest {
                plugin_id: "weather".to_string(),
                url: "https://example.com".to_string(),
                method: "GET".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_wildcard_matches_file() {
        let pattern = AllowancePattern::PluginWildcard {
            plugin_id: "weather".to_string(),
        };
        assert!(pattern.matches(
            &SensitiveAction::PluginFileAccess {
                plugin_id: "weather".to_string(),
                path: "/tmp/file".to_string(),
                mode: Permission::Read,
            },
            None
        ));
    }

    #[test]
    fn test_plugin_wildcard_wrong_plugin() {
        let pattern = AllowancePattern::PluginWildcard {
            plugin_id: "weather".to_string(),
        };
        assert!(!pattern.matches(
            &SensitiveAction::PluginExecution {
                plugin_id: "other".to_string(),
                capability: "anything".to_string(),
            },
            None
        ));
    }

    #[test]
    fn test_plugin_patterns_dont_match_non_plugin_actions() {
        let cap_pattern = AllowancePattern::PluginCapability {
            plugin_id: "test".to_string(),
            capability: "read".to_string(),
        };
        let wildcard_pattern = AllowancePattern::PluginWildcard {
            plugin_id: "test".to_string(),
        };
        let non_plugin = SensitiveAction::McpToolCall {
            server: "test".to_string(),
            tool: "read".to_string(),
        };
        assert!(!cap_pattern.matches(&non_plugin, None));
        assert!(!wildcard_pattern.matches(&non_plugin, None));

        let file_action = SensitiveAction::FileDelete {
            path: "/tmp/file".to_string(),
        };
        assert!(!cap_pattern.matches(&file_action, None));
        assert!(!wildcard_pattern.matches(&file_action, None));
    }

    #[test]
    fn test_plugin_pattern_display() {
        let pattern = AllowancePattern::PluginCapability {
            plugin_id: "weather".to_string(),
            capability: "http_request".to_string(),
        };
        assert_eq!(pattern.to_string(), "plugin://weather:http_request");

        let pattern = AllowancePattern::PluginWildcard {
            plugin_id: "weather".to_string(),
        };
        assert_eq!(pattern.to_string(), "plugin://weather:*");
    }

    #[test]
    fn test_plugin_pattern_serialization_roundtrip() {
        let patterns = vec![
            AllowancePattern::PluginCapability {
                plugin_id: "p1".to_string(),
                capability: "cap1".to_string(),
            },
            AllowancePattern::PluginWildcard {
                plugin_id: "p2".to_string(),
            },
        ];
        for pattern in patterns {
            let json = serde_json::to_string(&pattern).unwrap();
            let deserialized: AllowancePattern = serde_json::from_str(&json).unwrap();
            assert_eq!(pattern.to_string(), deserialized.to_string());
        }
    }
}
