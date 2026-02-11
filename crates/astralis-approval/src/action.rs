//! Sensitive action classification.
//!
//! [`SensitiveAction`] categorizes risky operations that may require
//! human approval before execution. Each variant captures the relevant
//! context needed for informed decision-making.

use astralis_core::types::{Permission, RiskLevel};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A sensitive action that may require human approval.
///
/// Each variant represents a category of operation with enough context
/// for a human to make an informed allow/deny decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SensitiveAction {
    /// Read a file or search the filesystem.
    ///
    /// Even read-only operations can expose sensitive data (credentials,
    /// private keys, personal information). All tool calls go through the
    /// interceptor so the policy and allowance system can gate access.
    FileRead {
        /// Path or pattern being read/searched.
        path: String,
    },

    /// Delete a file.
    FileDelete {
        /// Path to the file being deleted.
        path: String,
    },

    /// Write a file outside the operational workspace.
    FileWriteOutsideSandbox {
        /// Path to the file being written.
        path: String,
    },

    /// Execute a shell command.
    ExecuteCommand {
        /// The command to execute.
        command: String,
        /// Command arguments.
        args: Vec<String>,
    },

    /// Make a network request.
    NetworkRequest {
        /// Target host.
        host: String,
        /// Target port.
        port: u16,
    },

    /// Transmit data to an external destination.
    TransmitData {
        /// Where the data is being sent.
        destination: String,
        /// Type/classification of the data.
        data_type: String,
    },

    /// Perform a financial transaction.
    FinancialTransaction {
        /// Amount (as string to avoid floating-point issues).
        amount: String,
        /// Recipient identifier.
        recipient: String,
    },

    /// Change access control settings.
    AccessControlChange {
        /// The resource whose access is changing.
        resource: String,
        /// Description of the change.
        change: String,
    },

    /// Grant a capability token.
    CapabilityGrant {
        /// Resource pattern the capability covers.
        resource_pattern: String,
        /// Permissions being granted.
        permissions: Vec<Permission>,
    },

    /// Call an MCP tool that requires approval.
    McpToolCall {
        /// MCP server name.
        server: String,
        /// Tool name.
        tool: String,
    },
}

impl SensitiveAction {
    /// Get a short label for the action type.
    #[must_use]
    pub fn action_type(&self) -> &'static str {
        match self {
            Self::FileRead { .. } => "file_read",
            Self::FileDelete { .. } => "file_delete",
            Self::FileWriteOutsideSandbox { .. } => "file_write_outside_sandbox",
            Self::ExecuteCommand { .. } => "execute_command",
            Self::NetworkRequest { .. } => "network_request",
            Self::TransmitData { .. } => "transmit_data",
            Self::FinancialTransaction { .. } => "financial_transaction",
            Self::AccessControlChange { .. } => "access_control_change",
            Self::CapabilityGrant { .. } => "capability_grant",
            Self::McpToolCall { .. } => "mcp_tool_call",
        }
    }

    /// Get the default risk level for this action type.
    ///
    /// This provides a baseline; the actual risk assessment may be
    /// adjusted based on context (e.g., deleting a temp file vs a config file).
    #[must_use]
    pub fn default_risk_level(&self) -> RiskLevel {
        match self {
            Self::FinancialTransaction { .. } | Self::AccessControlChange { .. } => {
                RiskLevel::Critical
            },
            Self::FileDelete { .. }
            | Self::FileWriteOutsideSandbox { .. }
            | Self::ExecuteCommand { .. }
            | Self::TransmitData { .. }
            | Self::CapabilityGrant { .. } => RiskLevel::High,
            Self::FileRead { .. } | Self::NetworkRequest { .. } | Self::McpToolCall { .. } => {
                RiskLevel::Medium
            },
        }
    }

    /// Get a human-readable summary of the action.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::FileRead { path } => format!("Read: {path}"),
            Self::FileDelete { path } => format!("Delete file: {path}"),
            Self::FileWriteOutsideSandbox { path } => {
                format!("Write file outside workspace: {path}")
            },
            Self::ExecuteCommand { command, args } => {
                if args.is_empty() {
                    format!("Execute: {command}")
                } else {
                    format!("Execute: {command} {}", args.join(" "))
                }
            },
            Self::NetworkRequest { host, port } => format!("Network request to {host}:{port}"),
            Self::TransmitData {
                destination,
                data_type,
            } => format!("Transmit {data_type} to {destination}"),
            Self::FinancialTransaction { amount, recipient } => {
                format!("Financial transaction: {amount} to {recipient}")
            },
            Self::AccessControlChange { resource, change } => {
                format!("Access control change on {resource}: {change}")
            },
            Self::CapabilityGrant {
                resource_pattern,
                permissions,
            } => {
                let perms: Vec<_> = permissions.iter().map(ToString::to_string).collect();
                format!(
                    "Grant capability: [{}] on {resource_pattern}",
                    perms.join(", ")
                )
            },
            Self::McpToolCall { server, tool } => format!("MCP tool call: {server}/{tool}"),
        }
    }
}

impl fmt::Display for SensitiveAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_type_labels() {
        let action = SensitiveAction::FileDelete {
            path: "/tmp/test".to_string(),
        };
        assert_eq!(action.action_type(), "file_delete");

        let action = SensitiveAction::McpToolCall {
            server: "fs".to_string(),
            tool: "read".to_string(),
        };
        assert_eq!(action.action_type(), "mcp_tool_call");
    }

    #[test]
    fn test_default_risk_levels() {
        assert_eq!(
            SensitiveAction::FileDelete {
                path: String::new()
            }
            .default_risk_level(),
            RiskLevel::High
        );
        assert_eq!(
            SensitiveAction::FinancialTransaction {
                amount: String::new(),
                recipient: String::new()
            }
            .default_risk_level(),
            RiskLevel::Critical
        );
        assert_eq!(
            SensitiveAction::NetworkRequest {
                host: String::new(),
                port: 0
            }
            .default_risk_level(),
            RiskLevel::Medium
        );
    }

    #[test]
    fn test_action_summary() {
        let action = SensitiveAction::FileDelete {
            path: "/home/user/important.txt".to_string(),
        };
        assert_eq!(action.summary(), "Delete file: /home/user/important.txt");

        let action = SensitiveAction::ExecuteCommand {
            command: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/build".to_string()],
        };
        assert_eq!(action.summary(), "Execute: rm -rf /tmp/build");

        let action = SensitiveAction::CapabilityGrant {
            resource_pattern: "mcp://filesystem:*".to_string(),
            permissions: vec![Permission::Read, Permission::Write],
        };
        assert!(action.summary().contains("read, write"));
        assert!(action.summary().contains("mcp://filesystem:*"));
    }

    #[test]
    fn test_display_matches_summary() {
        let action = SensitiveAction::McpToolCall {
            server: "github".to_string(),
            tool: "create_issue".to_string(),
        };
        assert_eq!(action.to_string(), action.summary());
    }

    #[test]
    fn test_action_serialization() {
        let action = SensitiveAction::NetworkRequest {
            host: "api.example.com".to_string(),
            port: 443,
        };
        let json = serde_json::to_string(&action).unwrap();
        let deserialized: SensitiveAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action.action_type(), deserialized.action_type());
    }
}
