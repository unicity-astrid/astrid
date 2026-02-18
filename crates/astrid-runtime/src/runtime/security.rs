//! Security classification, approval bridging, and `FrontendApprovalHandler`.

use astrid_approval::manager::ApprovalHandler;
use astrid_approval::request::{
    ApprovalDecision as InternalApprovalDecision, ApprovalRequest as InternalApprovalRequest,
    ApprovalResponse as InternalApprovalResponse,
};
use astrid_approval::{InterceptProof, SensitiveAction};
use astrid_audit::AuthorizationProof;
use astrid_core::{ApprovalDecision, ApprovalOption, ApprovalRequest, Frontend};
use async_trait::async_trait;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// FrontendApprovalHandler — bridges Frontend::request_approval() to ApprovalHandler
// ---------------------------------------------------------------------------

/// Adapter that bridges a [`Frontend`] to the [`ApprovalHandler`] trait
/// used internally by the approval system.
pub(super) struct FrontendApprovalHandler<F: Frontend> {
    pub(super) frontend: Arc<F>,
}

#[async_trait]
impl<F: Frontend> ApprovalHandler for FrontendApprovalHandler<F> {
    async fn request_approval(
        &self,
        request: InternalApprovalRequest,
    ) -> Option<InternalApprovalResponse> {
        let frontend_request = to_frontend_request(&request);
        match self.frontend.request_approval(frontend_request).await {
            Ok(decision) => Some(to_internal_response(&request, &decision)),
            Err(_) => None,
        }
    }

    fn is_available(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Classify a tool call into a [`SensitiveAction`] for structured approval.
pub(super) fn classify_tool_call(
    server: &str,
    tool: &str,
    args: &serde_json::Value,
) -> SensitiveAction {
    let tool_lower = tool.to_lowercase();

    // File delete/remove operations
    if (tool_lower.contains("delete") || tool_lower.contains("remove"))
        && let Some(path) = args
            .get("path")
            .or_else(|| args.get("file"))
            .and_then(|v| v.as_str())
    {
        return SensitiveAction::FileDelete {
            path: path.to_string(),
        };
    }

    // Command execution
    if tool_lower.contains("exec") || tool_lower.contains("run") || tool_lower.contains("bash") {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or(tool)
            .to_string();
        let cmd_args = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        return SensitiveAction::ExecuteCommand {
            command,
            args: cmd_args,
        };
    }

    // File write outside workspace (detected by path args starting with / and outside cwd)
    if tool_lower.contains("write")
        && let Some(path) = args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
        && path.starts_with('/')
    {
        return SensitiveAction::FileWriteOutsideSandbox {
            path: path.to_string(),
        };
    }

    // Default: generic MCP tool call
    SensitiveAction::McpToolCall {
        server: server.to_string(),
        tool: tool.to_string(),
    }
}

/// Classify a built-in tool call into a [`SensitiveAction`].
///
/// Every tool — including read-only ones — goes through the interceptor because
/// even reads can expose sensitive data (credentials, private keys, PII).
pub(super) fn classify_builtin_tool_call(
    tool_name: &str,
    args: &serde_json::Value,
) -> SensitiveAction {
    match tool_name {
        "bash" => {
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("bash")
                .to_string();
            SensitiveAction::ExecuteCommand {
                command,
                args: Vec::new(),
            }
        },
        "write_file" | "edit_file" => {
            let path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            SensitiveAction::FileWriteOutsideSandbox { path }
        },
        "read_file" | "glob" | "grep" | "list_directory" => {
            let path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .or_else(|| args.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or(".")
                .to_string();
            SensitiveAction::FileRead { path }
        },
        // Unknown built-in tool — treat as MCP tool call requiring approval
        other => SensitiveAction::McpToolCall {
            server: "builtin".to_string(),
            tool: other.to_string(),
        },
    }
}

/// Convert an [`InterceptProof`] to an [`AuthorizationProof`] for audit logging.
///
/// Maps the interceptor's authorization decision to the audit trail's proof
/// format, preserving the actual authorization mechanism (policy, user approval,
/// capability, or allowance) rather than using a generic `System` proof.
pub(super) fn intercept_proof_to_auth_proof(
    proof: &InterceptProof,
    user_id: [u8; 8],
    context: &str,
) -> AuthorizationProof {
    match proof {
        InterceptProof::PolicyAllowed => AuthorizationProof::NotRequired {
            reason: format!("policy auto-approved: {context}"),
        },
        InterceptProof::UserApproval { approval_audit_id }
        | InterceptProof::CapabilityCreated {
            approval_audit_id, ..
        } => AuthorizationProof::UserApproval {
            user_id,
            approval_entry_id: approval_audit_id.clone(),
        },
        InterceptProof::SessionApproval { allowance_id } => AuthorizationProof::NotRequired {
            reason: format!("session-scoped allowance {allowance_id}: {context}"),
        },
        InterceptProof::WorkspaceApproval { allowance_id } => AuthorizationProof::NotRequired {
            reason: format!("workspace-scoped allowance {allowance_id}: {context}"),
        },
        InterceptProof::Capability { token_id } => AuthorizationProof::Capability {
            token_id: token_id.clone(),
            // InterceptProof only carries the token_id, not the full token bytes.
            // Hash the token_id string as a deterministic fingerprint so the audit
            // entry is at least tied to a specific token, even though we cannot
            // compute the true content hash without the full token.
            token_hash: astrid_crypto::ContentHash::hash(token_id.to_string().as_bytes()),
        },
        InterceptProof::Allowance { .. } => AuthorizationProof::NotRequired {
            reason: format!("pre-existing allowance: {context}"),
        },
    }
}

/// Convert an internal approval request to a frontend-facing [`ApprovalRequest`].
fn to_frontend_request(internal: &InternalApprovalRequest) -> ApprovalRequest {
    ApprovalRequest::new(
        internal.action.action_type().to_string(),
        internal.action.summary(),
    )
    .with_risk_level(internal.assessment.level)
    .with_resource(format!("{}", internal.action))
}

/// Convert a frontend [`ApprovalDecision`] to an internal [`ApprovalResponse`].
fn to_internal_response(
    request: &InternalApprovalRequest,
    decision: &ApprovalDecision,
) -> InternalApprovalResponse {
    let internal_decision = match decision.decision {
        ApprovalOption::AllowOnce => InternalApprovalDecision::Approve,
        ApprovalOption::AllowSession => InternalApprovalDecision::ApproveSession,
        ApprovalOption::AllowWorkspace => InternalApprovalDecision::ApproveWorkspace,
        ApprovalOption::AllowAlways => InternalApprovalDecision::ApproveAlways,
        ApprovalOption::Deny => InternalApprovalDecision::Deny {
            reason: decision
                .reason
                .clone()
                .unwrap_or_else(|| "denied by user".to_string()),
        },
    };
    InternalApprovalResponse::new(request.id.clone(), internal_decision)
}
