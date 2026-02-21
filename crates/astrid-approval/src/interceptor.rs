//! Security interceptor — combines all security layers.
//!
//! The [`SecurityInterceptor`] is the single entry point for all security checks.
//! It applies **intersection semantics**: both policy AND capability must allow
//! an action for it to proceed.
//!
//! # Security Check Flow
//!
//! 1. **Policy check** (hard boundaries — admin controls)
//!    - If blocked -> DENY immediately
//! 2. **Capability check** (does user/agent have a grant?)
//!    - If found -> use it as proof
//! 3. **Budget check** (is there remaining budget?)
//!    - If exceeded -> DENY or queue for override
//! 4. **Risk assessment / Approval** (how dangerous is this action?)
//!    - If high-risk and no capability -> request approval
//! 5. **Audit** — log the decision

use crate::error::{ApprovalError, ApprovalResult};
use astrid_audit::{
    AuditAction, AuditEntryId, AuditLog, AuditOutcome, AuthorizationProof as AuditAuthProof,
};
use astrid_capabilities::{CapabilityStore, CapabilityToken, ResourcePattern, TokenScope};
use astrid_core::types::{Permission, SessionId, TokenId};
use astrid_crypto::KeyPair;
use chrono::Duration;
use std::path::PathBuf;
use std::sync::Arc;

use crate::action::SensitiveAction;
use crate::allowance::{Allowance, AllowanceId, AllowancePattern, AllowanceStore};
use crate::budget::{BudgetResult, BudgetTracker, WorkspaceBudgetTracker};
use crate::manager::{ApprovalManager, ApprovalOutcome, ApprovalProof};
use crate::policy::{PolicyResult, SecurityPolicy};

/// Default TTL for "Allow Always" capability tokens (1 hour).
const ALLOW_ALWAYS_DEFAULT_TTL: Duration = Duration::hours(1);

/// Budget warning info to surface to the user.
#[derive(Debug, Clone)]
pub struct BudgetWarning {
    /// Current session spend (USD).
    pub current_spend: f64,
    /// Session budget (USD).
    pub session_max: f64,
    /// Percentage of budget used.
    pub percent_used: f64,
}

/// The result of a successful security intercept.
#[derive(Debug)]
pub struct InterceptResult {
    /// How the action was authorized.
    pub proof: InterceptProof,
    /// The audit entry ID for this decision.
    pub audit_id: AuditEntryId,
    /// Optional budget warning to surface to the user.
    pub budget_warning: Option<BudgetWarning>,
}

/// How an action was authorized through the interceptor.
#[derive(Debug)]
pub enum InterceptProof {
    /// Authorized by an existing capability token.
    Capability {
        /// Token ID that authorized the action.
        token_id: TokenId,
    },
    /// Authorized by an existing allowance.
    Allowance {
        /// Allowance ID that authorized the action.
        allowance_id: crate::allowance::AllowanceId,
    },
    /// Authorized by one-time user approval.
    UserApproval {
        /// Audit entry ID of the approval event.
        approval_audit_id: AuditEntryId,
    },
    /// Authorized by session approval.
    SessionApproval {
        /// Allowance ID created by the approval.
        allowance_id: crate::allowance::AllowanceId,
    },
    /// Authorized by workspace-scoped approval.
    WorkspaceApproval {
        /// Allowance ID created by the approval.
        allowance_id: crate::allowance::AllowanceId,
    },
    /// "Allow Always" — a new `CapabilityToken` was created and stored.
    ///
    /// Future requests for the same resource will be authorized by the
    /// `Capability` variant (existing token match) instead.
    CapabilityCreated {
        /// Token ID of the newly created capability.
        token_id: TokenId,
        /// Audit entry ID of the approval event (chain-link proof).
        approval_audit_id: AuditEntryId,
    },
    /// Policy allowed without further checks (low-risk, no approval needed).
    PolicyAllowed,
}

/// Security interceptor combining policy, capabilities, budget, and approval.
///
/// This is the single entry point for all security checks. All actions flow
/// through `intercept()` before execution.
pub struct SecurityInterceptor {
    /// Capability token store.
    capability_store: Arc<CapabilityStore>,
    /// Approval manager (allowances + handler + deferred queue).
    approval_manager: Arc<ApprovalManager>,
    /// Security policy (hard boundaries).
    policy: SecurityPolicy,
    /// Budget tracker.
    budget_tracker: Arc<BudgetTracker>,
    /// Audit log.
    audit_log: Arc<AuditLog>,
    /// Runtime signing key (for creating capability tokens on "Allow Always").
    runtime_key: Arc<KeyPair>,
    /// Current session ID for audit entries.
    session_id: SessionId,
    /// Allowance store for creating session/workspace allowances.
    allowance_store: Arc<AllowanceStore>,
    /// Workspace root for scoping workspace allowances.
    workspace_root: Option<PathBuf>,
    /// Workspace cumulative budget tracker (shared across sessions in a workspace).
    workspace_budget_tracker: Option<Arc<WorkspaceBudgetTracker>>,
}

impl SecurityInterceptor {
    /// Create a new security interceptor.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        capability_store: Arc<CapabilityStore>,
        approval_manager: Arc<ApprovalManager>,
        policy: SecurityPolicy,
        budget_tracker: Arc<BudgetTracker>,
        audit_log: Arc<AuditLog>,
        runtime_key: Arc<KeyPair>,
        session_id: SessionId,
        allowance_store: Arc<AllowanceStore>,
        workspace_root: Option<PathBuf>,
        workspace_budget_tracker: Option<Arc<WorkspaceBudgetTracker>>,
    ) -> Self {
        Self {
            capability_store,
            approval_manager,
            policy,
            budget_tracker,
            audit_log,
            runtime_key,
            session_id,
            allowance_store,
            workspace_root,
            workspace_budget_tracker,
        }
    }

    /// Intercept an action and determine if it should proceed.
    ///
    /// This is the main entry point. Applies intersection semantics:
    /// policy, capability, budget, and approval checks in sequence.
    ///
    /// # Errors
    ///
    /// Returns `ApprovalError` if the action is denied by policy, budget,
    /// or user decision.
    #[allow(clippy::too_many_lines)]
    pub async fn intercept(
        &self,
        action: &SensitiveAction,
        context: &str,
        estimated_cost: Option<f64>,
    ) -> ApprovalResult<InterceptResult> {
        // Step 1: Policy check (hard boundaries)
        let policy_result = self.policy.check(action);
        if let PolicyResult::Blocked { reason } = &policy_result {
            self.audit_denied(action, reason);
            return Err(ApprovalError::PolicyBlocked {
                tool: action.action_type().to_string(),
                reason: reason.clone(),
            });
        }

        // Step 2: Capability check
        if let Some(proof) = self.check_capability(action) {
            let mut cap_budget_warning = None;
            if let Some(cost) = estimated_cost {
                // Check workspace budget even for capability-authorized actions
                cap_budget_warning = self.check_workspace_budget(cost)?;
                // Atomic session budget check + reserve
                let session_result = self.budget_tracker.check_and_reserve(cost);
                if let BudgetResult::Exceeded {
                    reason,
                    requested,
                    available,
                } = session_result
                {
                    let deny_reason = format!(
                        "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                    );
                    self.audit_denied(action, &deny_reason);
                    return Err(ApprovalError::Denied {
                        reason: deny_reason,
                    });
                }
                if let BudgetResult::WarnAndAllow {
                    current_spend,
                    session_max,
                    percent_used,
                } = session_result
                {
                    cap_budget_warning = Some(BudgetWarning {
                        current_spend,
                        session_max,
                        percent_used,
                    });
                }
            }
            let audit_id = self.audit_allowed(action, &proof);
            return Ok(InterceptResult {
                proof,
                audit_id,
                budget_warning: cap_budget_warning,
            });
        }

        // Step 3: Budget check (atomic check + reserve)
        let mut budget_warning = None;
        if let Some(cost) = estimated_cost {
            // Workspace budget first (atomic check + reserve)
            budget_warning = self.check_workspace_budget(cost)?;

            // Session budget (atomic check + reserve)
            let budget_result = self.budget_tracker.check_and_reserve(cost);
            match budget_result {
                BudgetResult::Exceeded {
                    reason,
                    requested,
                    available,
                } => {
                    let deny_reason = format!(
                        "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                    );
                    self.audit_denied(action, &deny_reason);
                    return Err(ApprovalError::Denied {
                        reason: deny_reason,
                    });
                },
                BudgetResult::WarnAndAllow {
                    current_spend,
                    session_max,
                    percent_used,
                } => {
                    budget_warning = Some(BudgetWarning {
                        current_spend,
                        session_max,
                        percent_used,
                    });
                },
                BudgetResult::Allowed => {},
            }
        }

        // Step 4: Risk assessment / Approval
        // If policy says allowed (not requires_approval), and risk is low, skip approval
        // Budget was already reserved atomically in Step 3
        if matches!(policy_result, PolicyResult::Allowed) {
            let proof = InterceptProof::PolicyAllowed;
            let audit_id = self.audit_allowed(action, &proof);
            return Ok(InterceptResult {
                proof,
                audit_id,
                budget_warning,
            });
        }

        // Policy requires approval (or action has inherent risk) — go to approval manager
        let outcome = self
            .approval_manager
            .check_approval(action, context, self.workspace_root.as_deref())
            .await;

        match outcome {
            ApprovalOutcome::Allowed { proof } => {
                // Budget was already reserved atomically in Step 3
                let intercept_proof = match proof {
                    ApprovalProof::Allowance { allowance_id }
                    | ApprovalProof::CustomAllowance { allowance_id } => {
                        InterceptProof::Allowance { allowance_id }
                    },
                    ApprovalProof::OneTimeApproval => InterceptProof::UserApproval {
                        approval_audit_id: AuditEntryId::new(),
                    },
                    ApprovalProof::SessionApproval { .. } => {
                        self.create_allowance_for_action(action, true)
                    },
                    ApprovalProof::WorkspaceApproval { .. } => {
                        self.create_allowance_for_action(action, false)
                    },
                    ApprovalProof::AlwaysAllow => {
                        let mut result = self.handle_allow_always(action)?;
                        result.budget_warning = budget_warning;
                        return Ok(result);
                    },
                };
                let audit_id = self.audit_allowed(action, &intercept_proof);
                Ok(InterceptResult {
                    proof: intercept_proof,
                    audit_id,
                    budget_warning,
                })
            },
            ApprovalOutcome::Denied { reason } => {
                self.audit_denied(action, &reason);
                Err(ApprovalError::Denied { reason })
            },
            ApprovalOutcome::Deferred {
                resolution_id,
                fallback,
            } => {
                let reason =
                    format!("action deferred (resolution: {resolution_id}, fallback: {fallback})");
                self.audit_deferred(action, &reason);
                Err(ApprovalError::Timeout { timeout_ms: 0 })
            },
        }
    }

    /// Handle the "Allow Always" flow: create a `CapabilityToken` and store it.
    ///
    /// 1. Log approval to audit → get `approval_audit_id`
    /// 2. Convert action to `ResourcePattern` + permissions
    /// 3. Create `CapabilityToken` (persistent, 1h TTL, signed by runtime key)
    /// 4. Store in `CapabilityStore`
    /// 5. Return `InterceptProof::CapabilityCreated`
    fn handle_allow_always(&self, action: &SensitiveAction) -> ApprovalResult<InterceptResult> {
        // Step 1: Determine resource pattern and permission for the token
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

        // Step 2: Log the approval to audit → get the chain-link audit_id
        let approval_audit_id = {
            let audit_action = sensitive_action_to_audit(action);
            match self.audit_log.append(
                self.session_id.clone(),
                audit_action,
                AuditAuthProof::UserApproval {
                    user_id: self.runtime_key.key_id(),
                    approval_entry_id: AuditEntryId::new(), // self-referential placeholder
                },
                AuditOutcome::success(),
            ) {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!("failed to audit 'Allow Always' approval: {e}");
                    AuditEntryId::new()
                },
            }
        };

        // Step 3: Create the capability token
        // AuditEntryId is the same type (re-exported from astrid_capabilities)
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

        // Step 4: Store in CapabilityStore
        if let Err(e) = self.capability_store.add(token) {
            tracing::error!("failed to store 'Allow Always' capability token: {e}");
            // Fall back to one-time approval
            let proof = InterceptProof::UserApproval {
                approval_audit_id: approval_audit_id.clone(),
            };
            return Ok(InterceptResult {
                proof,
                audit_id: approval_audit_id,
                budget_warning: None,
            });
        }

        tracing::info!(
            %token_id,
            %resource_str,
            "created 'Allow Always' capability token (TTL: 1h)"
        );

        // Step 5: Audit the capability creation and return
        let proof = InterceptProof::CapabilityCreated {
            token_id,
            approval_audit_id: approval_audit_id.clone(),
        };
        let audit_id = self.audit_allowed(action, &proof);
        Ok(InterceptResult {
            proof,
            audit_id,
            budget_warning: None,
        })
    }

    /// Check the workspace budget atomically. Returns a budget warning if
    /// applicable, or an error if the budget is exceeded.
    fn check_workspace_budget(&self, cost: f64) -> Result<Option<BudgetWarning>, ApprovalError> {
        let Some(ref ws_budget) = self.workspace_budget_tracker else {
            return Ok(None);
        };
        match ws_budget.check_and_reserve(cost) {
            BudgetResult::Exceeded {
                reason,
                requested,
                available,
            } => Err(ApprovalError::Denied {
                reason: format!(
                    "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                ),
            }),
            BudgetResult::WarnAndAllow {
                current_spend,
                session_max,
                percent_used,
            } => Ok(Some(BudgetWarning {
                current_spend,
                session_max,
                percent_used,
            })),
            BudgetResult::Allowed => Ok(None),
        }
    }

    /// Check if an existing capability token covers this action.
    fn check_capability(&self, action: &SensitiveAction) -> Option<InterceptProof> {
        let (resource, permission) = action_to_resource_permission(action)?;
        let token = self
            .capability_store
            .find_capability(&resource, permission)?;
        Some(InterceptProof::Capability { token_id: token.id })
    }

    /// Create an allowance for a session or workspace approval.
    ///
    /// Maps the action to an `AllowancePattern`, creates an `Allowance`, stores it,
    /// and returns the appropriate `InterceptProof`. Falls back to `UserApproval`
    /// if the action cannot be mapped to a pattern (e.g. `FinancialTransaction`).
    fn create_allowance_for_action(
        &self,
        action: &SensitiveAction,
        session_only: bool,
    ) -> InterceptProof {
        let Some(pattern) = action_to_allowance_pattern(action) else {
            // Can't create a blanket allowance for this action type — fall back to one-time
            return InterceptProof::UserApproval {
                approval_audit_id: AuditEntryId::new(),
            };
        };

        // Audit the allowance creation
        let scope = if session_only { "session" } else { "workspace" };
        let audit_action = sensitive_action_to_audit(action);
        let _ = self.audit_log.append(
            self.session_id.clone(),
            audit_action,
            AuditAuthProof::UserApproval {
                user_id: self.runtime_key.key_id(),
                approval_entry_id: AuditEntryId::new(),
            },
            AuditOutcome::success_with(format!("allowance created ({scope})")),
        );

        let allowance_id = AllowanceId::new();
        let signature = self.runtime_key.sign(allowance_id.0.as_bytes());
        // Workspace allowances (session_only=false) get the workspace_root
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

        if let Err(e) = self.allowance_store.add_allowance(allowance) {
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

    /// Log an allowed action to the audit trail.
    fn audit_allowed(&self, action: &SensitiveAction, proof: &InterceptProof) -> AuditEntryId {
        let audit_action = sensitive_action_to_audit(action);
        let auth_proof = intercept_proof_to_audit(proof);

        match self.audit_log.append(
            self.session_id.clone(),
            audit_action,
            auth_proof,
            AuditOutcome::success(),
        ) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("failed to audit allowed action: {e}");
                AuditEntryId::new()
            },
        }
    }

    /// Log a denied action to the audit trail.
    fn audit_denied(&self, action: &SensitiveAction, reason: &str) {
        let audit_action = sensitive_action_to_audit(action);
        if let Err(e) = self.audit_log.append(
            self.session_id.clone(),
            audit_action,
            AuditAuthProof::Denied {
                reason: reason.to_string(),
            },
            AuditOutcome::failure(reason),
        ) {
            tracing::error!("failed to audit denied action: {e}");
        }
    }

    /// Log a deferred action to the audit trail.
    fn audit_deferred(&self, action: &SensitiveAction, reason: &str) {
        let audit_action = sensitive_action_to_audit(action);
        if let Err(e) = self.audit_log.append(
            self.session_id.clone(),
            audit_action,
            AuditAuthProof::Denied {
                reason: reason.to_string(),
            },
            AuditOutcome::failure(format!("deferred: {reason}")),
        ) {
            tracing::error!("failed to audit deferred action: {e}");
        }
    }

    /// Get a reference to the policy.
    #[must_use]
    pub fn policy(&self) -> &SecurityPolicy {
        &self.policy
    }

    /// Get a reference to the approval manager.
    #[must_use]
    pub fn approval_manager(&self) -> &ApprovalManager {
        &self.approval_manager
    }

    /// Get a reference to the budget tracker.
    #[must_use]
    pub fn budget_tracker(&self) -> &BudgetTracker {
        &self.budget_tracker
    }
}

impl std::fmt::Debug for SecurityInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecurityInterceptor")
            .field("policy", &self.policy)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Map a `SensitiveAction` to an `AllowancePattern` for session/workspace allowances.
///
/// Returns `None` for high-risk actions that should always require per-action approval
/// (e.g., financial transactions, access control changes).
fn action_to_allowance_pattern(action: &SensitiveAction) -> Option<AllowancePattern> {
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
        // Always require per-action approval — no blanket allowance
        SensitiveAction::TransmitData { .. }
        | SensitiveAction::FinancialTransaction { .. }
        | SensitiveAction::AccessControlChange { .. }
        | SensitiveAction::CapabilityGrant { .. } => None,
    }
}

/// Map a `SensitiveAction` to a resource string and permission for capability lookup.
fn action_to_resource_permission(action: &SensitiveAction) -> Option<(String, Permission)> {
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
        // These action types don't have a natural resource/permission mapping
        // for capability tokens — they always go through approval
        SensitiveAction::TransmitData { .. }
        | SensitiveAction::FinancialTransaction { .. }
        | SensitiveAction::AccessControlChange { .. }
        | SensitiveAction::CapabilityGrant { .. } => None,
    }
}

/// Convert a `SensitiveAction` to an `AuditAction`.
fn sensitive_action_to_audit(action: &SensitiveAction) -> AuditAction {
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
        SensitiveAction::PluginExecution {
            plugin_id,
            capability,
        } => AuditAction::ApprovalRequested {
            action_type: "plugin_execution".to_string(),
            resource: format!("plugin://{plugin_id}:{capability}"),
            risk_level: action.default_risk_level(),
        },
        SensitiveAction::PluginHttpRequest {
            plugin_id,
            url,
            method,
        } => AuditAction::ApprovalRequested {
            action_type: "plugin_http_request".to_string(),
            resource: format!("plugin://{plugin_id}:http_request ({method} {url})"),
            risk_level: action.default_risk_level(),
        },
        SensitiveAction::PluginFileAccess {
            plugin_id,
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
                action_type: "plugin_file_access".to_string(),
                resource: format!("plugin://{plugin_id}:{cap} ({path})"),
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

/// Convert an `InterceptProof` to an audit `AuthorizationProof`.
fn intercept_proof_to_audit(proof: &InterceptProof) -> AuditAuthProof {
    match proof {
        InterceptProof::Capability { token_id }
        | InterceptProof::CapabilityCreated {
            token_id,
            approval_audit_id: _,
        } => AuditAuthProof::Capability {
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
            user_id: [0u8; 8], // TODO: wire in actual user ID
            approval_entry_id: approval_audit_id.clone(),
        },
        InterceptProof::PolicyAllowed => AuditAuthProof::NotRequired {
            reason: "policy allowed".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowance::AllowanceStore;
    use crate::budget::BudgetConfig;
    use crate::deferred::DeferredResolutionStore;
    use crate::manager::ApprovalHandler;
    use crate::request::{ApprovalDecision, ApprovalRequest, ApprovalResponse};
    use astrid_crypto::KeyPair;

    /// Auto-approve handler for tests.
    struct AutoApproveHandler;

    #[async_trait::async_trait]
    impl ApprovalHandler for AutoApproveHandler {
        async fn request_approval(&self, request: ApprovalRequest) -> Option<ApprovalResponse> {
            Some(ApprovalResponse::new(request.id, ApprovalDecision::Approve))
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    /// Auto-deny handler for tests.
    struct AutoDenyHandler;

    #[async_trait::async_trait]
    impl ApprovalHandler for AutoDenyHandler {
        async fn request_approval(&self, request: ApprovalRequest) -> Option<ApprovalResponse> {
            Some(ApprovalResponse::new(
                request.id,
                ApprovalDecision::Deny {
                    reason: "test deny".to_string(),
                },
            ))
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    async fn make_interceptor(
        policy: SecurityPolicy,
        handler: Option<Arc<dyn ApprovalHandler>>,
    ) -> SecurityInterceptor {
        let audit_keypair = KeyPair::generate();
        let runtime_key = Arc::new(KeyPair::generate());
        let capability_store = Arc::new(CapabilityStore::in_memory());
        let allowance_store = Arc::new(AllowanceStore::new());
        let deferred_queue = Arc::new(DeferredResolutionStore::new());
        let approval_manager = Arc::new(ApprovalManager::new(
            Arc::clone(&allowance_store),
            deferred_queue,
        ));
        let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::new(100.0, 10.0)));
        let audit_log = Arc::new(AuditLog::in_memory(audit_keypair));
        let session_id = SessionId::new();

        let interceptor = SecurityInterceptor::new(
            capability_store,
            approval_manager,
            policy,
            budget_tracker,
            audit_log,
            runtime_key,
            session_id,
            allowance_store,
            None,
            None,
        );

        if let Some(h) = handler {
            interceptor.approval_manager.register_handler(h).await;
        }

        interceptor
    }

    // -----------------------------------------------------------------------
    // Policy blocked
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_blocked_by_policy() {
        let interceptor = make_interceptor(SecurityPolicy::default(), None).await;

        let action = SensitiveAction::ExecuteCommand {
            command: "sudo".to_string(),
            args: vec![],
        };
        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Allowed by policy (no approval needed)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_allowed_by_policy() {
        let interceptor = make_interceptor(
            SecurityPolicy::permissive(),
            Some(Arc::new(AutoApproveHandler)),
        )
        .await;

        let action = SensitiveAction::McpToolCall {
            server: "safe".to_string(),
            tool: "read".to_string(),
        };
        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_ok());
        assert!(matches!(
            result.unwrap().proof,
            InterceptProof::PolicyAllowed
        ));
    }

    // -----------------------------------------------------------------------
    // Requires approval — approved
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_requires_approval_approved() {
        let interceptor = make_interceptor(
            SecurityPolicy::default(),
            Some(Arc::new(AutoApproveHandler)),
        )
        .await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };
        let result = interceptor.intercept(&action, "cleanup", None).await;
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Requires approval — denied
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_requires_approval_denied() {
        let interceptor =
            make_interceptor(SecurityPolicy::default(), Some(Arc::new(AutoDenyHandler))).await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };
        let result = interceptor.intercept(&action, "cleanup", None).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Budget exceeded
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_budget_exceeded() {
        let interceptor = make_interceptor(
            SecurityPolicy::default(),
            Some(Arc::new(AutoApproveHandler)),
        )
        .await;

        // Per-action limit is 10.0
        let action = SensitiveAction::NetworkRequest {
            host: "api.example.com".to_string(),
            port: 443,
        };
        let result = interceptor
            .intercept(&action, "expensive call", Some(50.0))
            .await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Budget within limits
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_budget_within_limits() {
        let interceptor = make_interceptor(
            SecurityPolicy::default(),
            Some(Arc::new(AutoApproveHandler)),
        )
        .await;

        let action = SensitiveAction::NetworkRequest {
            host: "api.example.com".to_string(),
            port: 443,
        };
        let result = interceptor
            .intercept(&action, "cheap call", Some(5.0))
            .await;
        assert!(result.is_ok());
        // Cost should have been recorded
        assert!((interceptor.budget_tracker.spent() - 5.0).abs() < f64::EPSILON);
    }

    // -----------------------------------------------------------------------
    // Allow Always (creates CapabilityToken)
    // -----------------------------------------------------------------------

    /// Handler that returns "Allow Always" for all requests.
    struct AlwaysAlwaysHandler;

    #[async_trait::async_trait]
    impl ApprovalHandler for AlwaysAlwaysHandler {
        async fn request_approval(&self, request: ApprovalRequest) -> Option<ApprovalResponse> {
            Some(ApprovalResponse::new(
                request.id,
                ApprovalDecision::ApproveAlways,
            ))
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn test_allow_always_creates_capability() {
        let interceptor = make_interceptor(
            SecurityPolicy::default(),
            Some(Arc::new(AlwaysAlwaysHandler)),
        )
        .await;

        // FileDelete requires approval under the default policy
        let action = SensitiveAction::FileDelete {
            path: "/home/user/temp.txt".to_string(),
        };
        let result = interceptor.intercept(&action, "cleanup", None).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(matches!(
            result.proof,
            InterceptProof::CapabilityCreated { .. }
        ));

        // The capability token should now be in the store
        let found = interceptor
            .capability_store
            .find_capability("file:///home/user/temp.txt", Permission::Delete);
        assert!(found.is_some());

        // Second call should be authorized by the existing capability token (step 2)
        let result2 = interceptor.intercept(&action, "cleanup again", None).await;
        assert!(result2.is_ok());
        assert!(matches!(
            result2.unwrap().proof,
            InterceptProof::Capability { .. }
        ));
    }

    #[tokio::test]
    async fn test_allow_always_non_mappable_action_falls_back() {
        let interceptor = make_interceptor(
            SecurityPolicy::default(),
            Some(Arc::new(AlwaysAlwaysHandler)),
        )
        .await;

        // FinancialTransaction has no resource mapping → should error
        let action = SensitiveAction::FinancialTransaction {
            amount: "$100".to_string(),
            recipient: "vendor".to_string(),
        };
        let result = interceptor.intercept(&action, "paying vendor", None).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Debug
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Plugin interceptor tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_plugin_execution_auto_approve() {
        let interceptor = make_interceptor(
            SecurityPolicy::permissive(),
            Some(Arc::new(AutoApproveHandler)),
        )
        .await;

        let action = SensitiveAction::PluginExecution {
            plugin_id: "weather".to_string(),
            capability: "config_read".to_string(),
        };
        // Permissive policy still requires approval for plugins
        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_plugin_blocked_by_policy() {
        let mut policy = SecurityPolicy::permissive();
        policy.blocked_plugins.insert("evil-plugin".to_string());

        let interceptor = make_interceptor(policy, Some(Arc::new(AutoApproveHandler))).await;

        let action = SensitiveAction::PluginExecution {
            plugin_id: "evil-plugin".to_string(),
            capability: "anything".to_string(),
        };
        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plugin_allow_always_creates_capability() {
        let interceptor = make_interceptor(
            SecurityPolicy::default(),
            Some(Arc::new(AlwaysAlwaysHandler)),
        )
        .await;

        let action = SensitiveAction::PluginExecution {
            plugin_id: "weather".to_string(),
            capability: "config_read".to_string(),
        };
        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(matches!(
            result.proof,
            InterceptProof::CapabilityCreated { .. }
        ));

        // Verify the capability is stored and can be looked up
        let found = interceptor
            .capability_store
            .find_capability("plugin://weather:config_read", Permission::Invoke);
        assert!(found.is_some());

        // Second call should use the existing capability
        let result2 = interceptor.intercept(&action, "test again", None).await;
        assert!(result2.is_ok());
        assert!(matches!(
            result2.unwrap().proof,
            InterceptProof::Capability { .. }
        ));
    }

    #[tokio::test]
    async fn test_plugin_http_denied_host_blocked() {
        let mut policy = SecurityPolicy::permissive();
        policy.denied_hosts.push("evil.com".to_string());

        let interceptor = make_interceptor(policy, Some(Arc::new(AutoApproveHandler))).await;

        let action = SensitiveAction::PluginHttpRequest {
            plugin_id: "weather".to_string(),
            url: "https://evil.com/api".to_string(),
            method: "GET".to_string(),
        };
        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plugin_file_denied_path_blocked() {
        let mut policy = SecurityPolicy::permissive();
        policy.denied_paths.push("/etc/**".to_string());

        let interceptor = make_interceptor(policy, Some(Arc::new(AutoApproveHandler))).await;

        let action = SensitiveAction::PluginFileAccess {
            plugin_id: "cache".to_string(),
            path: "/etc/passwd".to_string(),
            mode: Permission::Read,
        };
        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Debug
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_debug() {
        let interceptor = make_interceptor(
            SecurityPolicy::default(),
            Some(Arc::new(AutoApproveHandler)),
        )
        .await;
        let debug = format!("{interceptor:?}");
        assert!(debug.contains("SecurityInterceptor"));
    }

    // -----------------------------------------------------------------------
    // Capability check
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_capability_check() {
        use astrid_capabilities::{
            AuditEntryId as CapAuditId, CapabilityToken, ResourcePattern, TokenScope,
        };
        let keypair = KeyPair::generate();
        let capability_store = Arc::new(CapabilityStore::in_memory());

        // Add a capability token for the tool
        let pattern = ResourcePattern::new("mcp://filesystem:read_file").unwrap();
        let token = CapabilityToken::create(
            pattern,
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            CapAuditId::new(),
            &keypair,
            None,
        );
        capability_store.add(token).unwrap();

        let allowance_store = Arc::new(AllowanceStore::new());
        let deferred_queue = Arc::new(DeferredResolutionStore::new());
        let approval_manager = Arc::new(ApprovalManager::new(
            Arc::clone(&allowance_store),
            deferred_queue,
        ));
        let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::new(100.0, 10.0)));
        let audit_log = Arc::new(AuditLog::in_memory(KeyPair::generate()));

        let runtime_key = Arc::new(KeyPair::generate());
        let interceptor = SecurityInterceptor::new(
            capability_store,
            approval_manager,
            SecurityPolicy::default(),
            budget_tracker,
            audit_log,
            runtime_key,
            SessionId::new(),
            allowance_store,
            None,
            None,
        );

        let action = SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        };
        let result = interceptor.intercept(&action, "reading file", None).await;
        assert!(result.is_ok());
        assert!(matches!(
            result.unwrap().proof,
            InterceptProof::Capability { .. }
        ));
    }
}
