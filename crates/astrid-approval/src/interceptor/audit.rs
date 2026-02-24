use super::types::InterceptProof;
use crate::action::SensitiveAction;
use astrid_audit::{AuditAction, AuthorizationProof as AuditAuthProof};
use astrid_core::types::Permission;

/// Converts a generic sensitive action struct to an exact auditable string map payload.
#[must_use]
pub fn sensitive_action_to_audit(action: &SensitiveAction) -> AuditAction {
    match action {
        SensitiveAction::McpToolCall { server, tool } => AuditAction::McpToolCall {
            server: server.clone(),
            tool: tool.clone(),
            args_hash: astrid_crypto::ContentHash::zero(),
        },
        SensitiveAction::FileDelete { path } => AuditAction::FileDelete { path: path.clone() },
        SensitiveAction::FileWriteOutsideSandbox { path } => AuditAction::FileWrite {
            path: path.clone(),
            content_hash: astrid_crypto::ContentHash::zero(),
        },
        SensitiveAction::ExecuteCommand { command, args } => AuditAction::ApprovalRequested {
            action_type: "execute_command".to_string(),
            resource: format!("{command} {}", args.join(" ")),
            risk_level: action.default_risk_level(),
        },
        SensitiveAction::NetworkRequest { host, port } => AuditAction::ApprovalRequested {
            action_type: "network_request".to_string(),
            resource: format!("{host}:{port}"),
            risk_level: action.default_risk_level(),
        },
        SensitiveAction::CapsuleExecution {
            capsule_id,
            capability,
        } => AuditAction::ApprovalRequested {
            action_type: "capsule_execution".to_string(),
            resource: format!("capsule://{capsule_id}:{capability}"),
            risk_level: action.default_risk_level(),
        },
        SensitiveAction::CapsuleHttpRequest {
            capsule_id,
            url,
            method,
        } => AuditAction::ApprovalRequested {
            action_type: "capsule_http_request".to_string(),
            resource: format!("capsule://{capsule_id}:http_request ({method} {url})"),
            risk_level: action.default_risk_level(),
        },
        SensitiveAction::CapsuleFileAccess {
            capsule_id,
            path,
            mode,
        } => {
            let cap = match mode {
                Permission::Read => "file_read",
                Permission::Write => "file_write",
                Permission::Delete => "file_delete",
                _ => "file_access",
            };
            AuditAction::ApprovalRequested {
                action_type: "capsule_file_access".to_string(),
                resource: format!("capsule://{capsule_id}:{cap} ({path})"),
                risk_level: action.default_risk_level(),
            }
        },
        _ => AuditAction::ApprovalRequested {
            action_type: action.action_type().to_string(),
            resource: action.summary(),
            risk_level: action.default_risk_level(),
        },
    }
}

/// Converts an internal intercept proof into a serializable authorization proof format for the global audit log.
#[must_use]
pub fn intercept_proof_to_audit(proof: &InterceptProof, user_id: [u8; 8]) -> AuditAuthProof {
    match proof {
        InterceptProof::Capability { token_id }
        | InterceptProof::CapabilityCreated { token_id, .. } => AuditAuthProof::Capability {
            token_id: token_id.clone(),
            token_hash: astrid_crypto::ContentHash::zero(),
        },
        InterceptProof::Allowance { .. }
        | InterceptProof::SessionApproval { .. }
        | InterceptProof::WorkspaceApproval { .. } => AuditAuthProof::NotRequired {
            reason: "covered by allowance".to_string(),
        },
        InterceptProof::UserApproval {
            approval_audit_id, ..
        } => AuditAuthProof::UserApproval {
            user_id,
            approval_entry_id: Some(approval_audit_id.clone()),
        },
        InterceptProof::PolicyAllowed => AuditAuthProof::NotRequired {
            reason: "policy allowed".to_string(),
        },
    }
}
