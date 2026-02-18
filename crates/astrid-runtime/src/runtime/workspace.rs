//! Workspace boundary enforcement and path extraction helpers.

use astrid_audit::{AuditAction, AuditOutcome, AuthorizationProof};
use astrid_capabilities::AuditEntryId;
use astrid_core::{ApprovalOption, ApprovalRequest, Frontend, RiskLevel};
use astrid_llm::{LlmProvider, ToolCall, ToolCallResult};
use astrid_workspace::{EscapeDecision, EscapeRequest, PathCheck};
use std::path::PathBuf;
use tracing::{debug, info, warn};

use crate::session::AgentSession;

use super::AgentRuntime;

impl<P: LlmProvider + 'static> AgentRuntime<P> {
    /// Check workspace boundaries for a tool call's file path arguments.
    ///
    /// Returns `Ok(())` if all paths are allowed, or a tool error result if blocked/denied.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn check_workspace_boundaries<F: Frontend>(
        &self,
        session: &mut AgentSession,
        call: &ToolCall,
        server: &str,
        tool: &str,
        frontend: &F,
    ) -> Result<(), ToolCallResult> {
        let paths = extract_paths_from_args(&call.arguments);
        if paths.is_empty() {
            return Ok(());
        }

        for path in &paths {
            // Check escape handler first (already approved paths)
            if session.escape_handler.is_allowed(path) {
                debug!(path = %path.display(), "Path already approved by escape handler");
                continue;
            }

            let check = self.boundary.check(path);
            match check {
                PathCheck::Allowed | PathCheck::AutoAllowed => {},
                PathCheck::NeverAllowed => {
                    warn!(
                        path = %path.display(),
                        tool = %format!("{server}:{tool}"),
                        "Access to protected path blocked"
                    );

                    // Audit the blocked access
                    {
                        let _ = self.audit.append(
                            session.id.clone(),
                            AuditAction::ApprovalDenied {
                                action: format!("{server}:{tool} -> {}", path.display()),
                                reason: Some("protected system path".to_string()),
                            },
                            AuthorizationProof::System {
                                reason: "workspace boundary: never-allowed path".to_string(),
                            },
                            AuditOutcome::failure("protected path"),
                        );
                    }

                    return Err(ToolCallResult::error(
                        &call.id,
                        format!(
                            "Access to {} is blocked — this is a protected system path",
                            path.display()
                        ),
                    ));
                },
                PathCheck::RequiresApproval => {
                    let escape_request = EscapeRequest::new(
                        path.clone(),
                        infer_operation(tool),
                        format!(
                            "Tool {server}:{tool} wants to access {} outside the workspace",
                            path.display()
                        ),
                    )
                    .with_tool(tool)
                    .with_server(server);

                    // Bridge to frontend approval
                    let approval_request = ApprovalRequest::new(
                        format!("workspace-escape:{server}:{tool}"),
                        format!(
                            "Allow {} {} outside workspace?\n  Path: {}",
                            tool,
                            escape_request.operation,
                            path.display()
                        ),
                    )
                    .with_risk_level(risk_level_for_operation(escape_request.operation))
                    .with_resource(path.display().to_string());

                    let decision =
                        frontend
                            .request_approval(approval_request)
                            .await
                            .map_err(|_| {
                                ToolCallResult::error(
                                    &call.id,
                                    "Failed to request workspace escape approval",
                                )
                            })?;

                    // Convert ApprovalDecision to EscapeDecision
                    let escape_decision = match decision.decision {
                        ApprovalOption::AllowOnce => EscapeDecision::AllowOnce,
                        ApprovalOption::AllowSession | ApprovalOption::AllowWorkspace => {
                            EscapeDecision::AllowSession
                        },
                        ApprovalOption::AllowAlways => EscapeDecision::AllowAlways,
                        ApprovalOption::Deny => EscapeDecision::Deny,
                    };

                    // Record the decision in the escape handler
                    session
                        .escape_handler
                        .process_decision(&escape_request, escape_decision);

                    // Audit the decision
                    if escape_decision.is_allowed() {
                        let _ = self.audit.append(
                            session.id.clone(),
                            AuditAction::ApprovalGranted {
                                action: format!("{server}:{tool}"),
                                resource: Some(path.display().to_string()),
                                scope: match decision.decision {
                                    ApprovalOption::AllowSession => {
                                        astrid_audit::ApprovalScope::Session
                                    },
                                    ApprovalOption::AllowWorkspace => {
                                        astrid_audit::ApprovalScope::Workspace
                                    },
                                    ApprovalOption::AllowAlways => {
                                        astrid_audit::ApprovalScope::Always
                                    },
                                    ApprovalOption::AllowOnce | ApprovalOption::Deny => {
                                        astrid_audit::ApprovalScope::Once
                                    },
                                },
                            },
                            AuthorizationProof::UserApproval {
                                user_id: session.user_id,
                                approval_entry_id: AuditEntryId::new(),
                            },
                            AuditOutcome::success(),
                        );
                    } else {
                        let _ = self.audit.append(
                            session.id.clone(),
                            AuditAction::ApprovalDenied {
                                action: format!("{server}:{tool} -> {}", path.display()),
                                reason: Some(
                                    decision
                                        .reason
                                        .clone()
                                        .unwrap_or_else(|| "user denied".to_string()),
                                ),
                            },
                            AuthorizationProof::UserApproval {
                                user_id: session.user_id,
                                approval_entry_id: AuditEntryId::new(),
                            },
                            AuditOutcome::failure("user denied workspace escape"),
                        );
                    }

                    if !escape_decision.is_allowed() {
                        return Err(ToolCallResult::error(
                            &call.id,
                            decision.reason.unwrap_or_else(|| {
                                format!("Access to {} denied — outside workspace", path.display())
                            }),
                        ));
                    }

                    info!(
                        path = %path.display(),
                        decision = ?escape_decision,
                        "Workspace escape approved"
                    );
                },
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Extract file paths from tool call JSON arguments.
///
/// Scans for common path-like keys and string values that look like file paths.
pub(super) fn extract_paths_from_args(args: &serde_json::Value) -> Vec<PathBuf> {
    /// Keys commonly used for file path arguments in MCP tools.
    const PATH_KEYS: &[&str] = &[
        "path",
        "file",
        "file_path",
        "filepath",
        "filename",
        "directory",
        "dir",
        "target",
        "source",
        "destination",
        "src",
        "dst",
        "input",
        "output",
        "uri",
        "url",
        "cwd",
        "working_directory",
    ];

    let mut paths = Vec::new();

    if let Some(obj) = args.as_object() {
        for (key, value) in obj {
            let key_lower = key.to_lowercase();
            if let Some(s) = value.as_str()
                && PATH_KEYS.contains(&key_lower.as_str())
                && let Some(path) = try_extract_path(s)
            {
                paths.push(path);
            }
        }
    }

    paths
}

/// Try to interpret a string value as a file path.
fn try_extract_path(value: &str) -> Option<PathBuf> {
    // Handle file:// URIs
    if let Some(stripped) = value.strip_prefix("file://") {
        return Some(PathBuf::from(stripped));
    }

    // Skip non-file URIs
    if value.contains("://") {
        return None;
    }

    // Check if it looks like an absolute or relative file path
    if value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
    {
        return Some(PathBuf::from(value));
    }

    None
}

/// Infer the operation type from a tool name.
pub(super) fn infer_operation(tool: &str) -> astrid_workspace::escape::EscapeOperation {
    use astrid_workspace::escape::EscapeOperation;
    let tool_lower = tool.to_lowercase();

    if tool_lower.contains("read") || tool_lower.contains("get") || tool_lower.contains("cat") {
        EscapeOperation::Read
    } else if tool_lower.contains("write")
        || tool_lower.contains("set")
        || tool_lower.contains("put")
        || tool_lower.contains("edit")
        || tool_lower.contains("update")
    {
        EscapeOperation::Write
    } else if tool_lower.contains("create")
        || tool_lower.contains("mkdir")
        || tool_lower.contains("touch")
        || tool_lower.contains("new")
    {
        EscapeOperation::Create
    } else if tool_lower.contains("delete")
        || tool_lower.contains("remove")
        || tool_lower.contains("rm")
    {
        EscapeOperation::Delete
    } else if tool_lower.contains("exec")
        || tool_lower.contains("run")
        || tool_lower.contains("launch")
    {
        EscapeOperation::Execute
    } else if tool_lower.contains("list") || tool_lower.contains("ls") || tool_lower.contains("dir")
    {
        EscapeOperation::List
    } else {
        // Default to Read for unknown operations (least destructive assumption)
        EscapeOperation::Read
    }
}

/// Determine risk level based on the escape operation.
pub(super) fn risk_level_for_operation(
    operation: astrid_workspace::escape::EscapeOperation,
) -> RiskLevel {
    use astrid_workspace::escape::EscapeOperation;
    match operation {
        EscapeOperation::Read | EscapeOperation::List => RiskLevel::Medium,
        EscapeOperation::Write | EscapeOperation::Create => RiskLevel::High,
        EscapeOperation::Delete | EscapeOperation::Execute => RiskLevel::Critical,
    }
}
