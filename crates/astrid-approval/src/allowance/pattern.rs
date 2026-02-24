//! Allowance pattern types and matching logic.

use astrid_core::types::Permission;
use globset::Glob;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

use crate::action::SensitiveAction;

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
    /// For `CapsuleExecution`, matches on the `capability` field directly.
    /// For `CapsuleHttpRequest`, matches on `"http_request"`.
    /// For `CapsuleFileAccess`, matches on the derived capability name
    /// (`"file_read"`, `"file_write"`, `"file_delete"`).
    CapsuleCapability {
        /// Plugin identifier.
        capsule_id: String,
        /// Capability name to match.
        capability: String,
    },

    /// Match any action from a specific plugin (wildcard).
    CapsuleWildcard {
        /// Plugin identifier.
        capsule_id: String,
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

            // CapsuleCapability: match plugin actions with same capsule_id + derived capability
            (
                Self::CapsuleCapability {
                    capsule_id,
                    capability,
                },
                SensitiveAction::CapsuleExecution {
                    capsule_id: action_pid,
                    capability: action_cap,
                },
            ) => capsule_id == action_pid && capability == action_cap,

            (
                Self::CapsuleCapability {
                    capsule_id,
                    capability,
                },
                SensitiveAction::CapsuleHttpRequest {
                    capsule_id: action_pid,
                    ..
                },
            ) => capsule_id == action_pid && capability == "http_request",

            (
                Self::CapsuleCapability {
                    capsule_id,
                    capability,
                },
                SensitiveAction::CapsuleFileAccess {
                    capsule_id: action_pid,
                    mode,
                    ..
                },
            ) => {
                capsule_id == action_pid
                    && capability
                        == match mode {
                            Permission::Read => "file_read",
                            Permission::Write => "file_write",
                            Permission::Delete => "file_delete",
                            _ => return false,
                        }
            },

            // CapsuleWildcard: match any plugin action from the same capsule_id
            (
                Self::CapsuleWildcard { capsule_id },
                SensitiveAction::CapsuleExecution {
                    capsule_id: action_pid,
                    ..
                }
                | SensitiveAction::CapsuleHttpRequest {
                    capsule_id: action_pid,
                    ..
                }
                | SensitiveAction::CapsuleFileAccess {
                    capsule_id: action_pid,
                    ..
                },
            ) => capsule_id == action_pid,

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
            Self::CapsuleCapability {
                capsule_id,
                capability,
            } => write!(f, "capsule://{capsule_id}:{capability}"),
            Self::CapsuleWildcard { capsule_id } => write!(f, "capsule://{capsule_id}:*"),
        }
    }
}

#[cfg(test)]
#[path = "pattern_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pattern_plugin_tests.rs"]
mod plugin_tests;
