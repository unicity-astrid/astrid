use astrid_capabilities::{CapabilityStore, CapabilityToken, ResourcePattern, TokenScope};
use astrid_core::types::Permission;
use astrid_core::types::TokenId;
use astrid_crypto::KeyPair;
use std::sync::Arc;

use super::types::ALLOW_ALWAYS_DEFAULT_TTL;
use super::types::InterceptProof;
use crate::action::SensitiveAction;
use crate::error::{ApprovalError, ApprovalResult};

pub struct CapabilityValidator {
    pub store: Arc<CapabilityStore>,
    pub runtime_key: Arc<KeyPair>,
}

impl CapabilityValidator {
    pub fn new(store: Arc<CapabilityStore>, runtime_key: Arc<KeyPair>) -> Self {
        Self { store, runtime_key }
    }

    pub fn check_capability(&self, action: &SensitiveAction) -> Option<InterceptProof> {
        let (resource, permission) = action_to_resource_permission(action)?;
        let token = self.store.find_capability(&resource, permission)?;
        Some(InterceptProof::Capability { token_id: token.id })
    }

    pub fn handle_allow_always(
        &self,
        action: &SensitiveAction,
        approval_audit_id: astrid_capabilities::AuditEntryId,
    ) -> ApprovalResult<InterceptProof> {
        let (resource_str, permission) =
            action_to_resource_permission(action).ok_or_else(|| ApprovalError::Denied {
                reason: format!(
                    "cannot create 'Allow Always' capability for {}: no resource mapping",
                    action.action_type()
                ),
            })?;

        let resource = ResourcePattern::new(&resource_str).map_err(|e| ApprovalError::Denied {
            reason: format!("invalid resource pattern for capability: {e}"),
        })?;

        let token = CapabilityToken::create(
            resource,
            vec![permission],
            TokenScope::Persistent,
            self.runtime_key.key_id(),
            approval_audit_id.clone(),
            &self.runtime_key,
            Some(ALLOW_ALWAYS_DEFAULT_TTL),
        );
        let token_id = token.id.clone();

        if let Err(e) = self.store.add(token) {
            tracing::error!("failed to store 'Allow Always' capability token: {e}");
            return Ok(InterceptProof::UserApproval { approval_audit_id });
        }

        tracing::info!(%token_id, %resource_str, "created 'Allow Always' capability token (TTL: 1h)");
        Ok(InterceptProof::CapabilityCreated {
            token_id,
            approval_audit_id,
        })
    }
}

pub fn action_to_resource_permission(action: &SensitiveAction) -> Option<(String, Permission)> {
    match action {
        SensitiveAction::McpToolCall { server, tool } => {
            Some((format!("mcp://{server}:{tool}"), Permission::Invoke))
        },
        SensitiveAction::FileRead { path } => Some((format!("file://{path}"), Permission::Read)),
        SensitiveAction::FileDelete { path } => {
            Some((format!("file://{path}"), Permission::Delete))
        },
        SensitiveAction::FileWriteOutsideSandbox { path } => {
            Some((format!("file://{path}"), Permission::Write))
        },
        SensitiveAction::ExecuteCommand { command, .. } => {
            Some((format!("exec://{command}"), Permission::Execute))
        },
        SensitiveAction::NetworkRequest { host, port } => {
            Some((format!("net://{host}:{port}"), Permission::Invoke))
        },
        SensitiveAction::PluginExecution {
            plugin_id,
            capability,
        } => Some((
            format!("plugin://{plugin_id}:{capability}"),
            Permission::Invoke,
        )),
        SensitiveAction::PluginHttpRequest { plugin_id, .. } => Some((
            format!("plugin://{plugin_id}:http_request"),
            Permission::Invoke,
        )),
        SensitiveAction::PluginFileAccess {
            plugin_id, mode, ..
        } => {
            let cap = match mode {
                Permission::Read => "file_read",
                Permission::Write => "file_write",
                Permission::Delete => "file_delete",
                _ => return None,
            };
            Some((format!("plugin://{plugin_id}:{cap}"), Permission::Invoke))
        },
        _ => None,
    }
}
