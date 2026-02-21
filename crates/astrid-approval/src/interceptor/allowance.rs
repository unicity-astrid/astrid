use astrid_audit::AuditEntryId;
use astrid_core::types::Permission;
use astrid_crypto::KeyPair;
use std::path::PathBuf;
use std::sync::Arc;

use super::types::InterceptProof;
use crate::action::SensitiveAction;
use crate::allowance::{Allowance, AllowanceId, AllowancePattern, AllowanceStore};

/// Validates if an action is permitted by a given workspace allowance.
pub struct AllowanceValidator {
    /// The backing allowance store definition mappings.
    pub store: Arc<AllowanceStore>,
    /// The runtime verification keys for token validation.
    pub runtime_key: Arc<KeyPair>,
    /// Optional workspace root restriction for allowances.
    pub workspace_root: Option<PathBuf>,
}

impl AllowanceValidator {
    /// Creates a new `AllowanceValidator`.
    pub fn new(
        store: Arc<AllowanceStore>,
        runtime_key: Arc<KeyPair>,
        workspace_root: Option<PathBuf>,
    ) -> Self {
        Self {
            store,
            runtime_key,
            workspace_root,
        }
    }

    /// Creates a new allowance token based on a specific action intent.
    pub fn create_allowance_for_action(
        &self,
        action: &SensitiveAction,
        session_only: bool,
    ) -> InterceptProof {
        let Some(pattern) = action_to_allowance_pattern(action) else {
            return InterceptProof::UserApproval {
                approval_audit_id: AuditEntryId::new(), // To be replaced in caller if needed
            };
        };

        let allowance_id = AllowanceId::new();
        let signature = self.runtime_key.sign(allowance_id.0.as_bytes());
        let ws_root = if session_only {
            None
        } else {
            self.workspace_root.clone()
        };

        let allowance = Allowance {
            id: allowance_id.clone(),
            action_pattern: pattern,
            created_at: astrid_core::types::Timestamp::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            session_only,
            workspace_root: ws_root,
            signature,
        };

        if let Err(e) = self.store.add_allowance(allowance) {
            tracing::warn!("failed to store allowance: {e}");
            return InterceptProof::UserApproval {
                approval_audit_id: AuditEntryId::new(),
            };
        }

        if session_only {
            InterceptProof::SessionApproval { allowance_id }
        } else {
            InterceptProof::WorkspaceApproval { allowance_id }
        }
    }
}

/// Extracts the exact resource permission and pattern required to execute a specific action.
#[must_use] 
pub fn action_to_allowance_pattern(action: &SensitiveAction) -> Option<AllowancePattern> {
    match action {
        SensitiveAction::McpToolCall { server, tool } => Some(AllowancePattern::ExactTool {
            server: server.clone(),
            tool: tool.clone(),
        }),
        SensitiveAction::FileRead { path } => Some(AllowancePattern::FilePattern {
            pattern: path.clone(),
            permission: Permission::Read,
        }),
        SensitiveAction::FileDelete { path } => Some(AllowancePattern::FilePattern {
            pattern: path.clone(),
            permission: Permission::Delete,
        }),
        SensitiveAction::FileWriteOutsideSandbox { path } => Some(AllowancePattern::FilePattern {
            pattern: path.clone(),
            permission: Permission::Write,
        }),
        SensitiveAction::ExecuteCommand { command, .. } => Some(AllowancePattern::CommandPattern {
            command: command.clone(),
        }),
        SensitiveAction::NetworkRequest { host, port } => Some(AllowancePattern::NetworkHost {
            host: host.clone(),
            ports: Some(vec![*port]),
        }),
        SensitiveAction::PluginExecution {
            plugin_id,
            capability,
        } => Some(AllowancePattern::PluginCapability {
            plugin_id: plugin_id.clone(),
            capability: capability.clone(),
        }),
        SensitiveAction::PluginHttpRequest { plugin_id, .. } => {
            Some(AllowancePattern::PluginCapability {
                plugin_id: plugin_id.clone(),
                capability: "http_request".to_string(),
            })
        },
        SensitiveAction::PluginFileAccess {
            plugin_id, mode, ..
        } => {
            let cap = match mode {
                Permission::Read => "file_read",
                Permission::Write => "file_write",
                Permission::Delete => "file_delete",
                _ => return None,
            };
            Some(AllowancePattern::PluginCapability {
                plugin_id: plugin_id.clone(),
                capability: cap.to_string(),
            })
        },
        SensitiveAction::TransmitData { .. }
        | SensitiveAction::FinancialTransaction { .. }
        | SensitiveAction::AccessControlChange { .. }
        | SensitiveAction::CapabilityGrant { .. } => None,
    }
}
