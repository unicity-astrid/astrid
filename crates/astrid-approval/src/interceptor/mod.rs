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

/// Workspace sandboxing allowances.
pub mod allowance;
/// Audit logging integrations.
pub mod audit;
/// Budget enforcement integrations.
pub mod budget;
/// Capability token verification.
pub mod capability;
/// Types shared across interceptors.
pub mod types;

pub(crate) use allowance::AllowanceValidator;
pub(crate) use budget::BudgetValidator;
pub(crate) use capability::CapabilityValidator;
pub use types::*;

use crate::error::{ApprovalError, ApprovalResult};
use astrid_audit::{AuditEntryId, AuditLog, AuditOutcome, AuthorizationProof as AuditAuthProof};
use astrid_capabilities::CapabilityStore;
use astrid_core::principal::PrincipalId;
use astrid_core::types::SessionId;
use astrid_crypto::KeyPair;
use std::path::PathBuf;
use std::sync::Arc;

use crate::action::SensitiveAction;
use crate::allowance::AllowanceStore;
use crate::budget::{BudgetTracker, WorkspaceBudgetTracker};
use crate::interceptor::audit::{intercept_proof_to_audit, sensitive_action_to_audit};
use crate::manager::{ApprovalManager, ApprovalOutcome, ApprovalProof};
use crate::policy::{PolicyResult, SecurityPolicy};

/// Security interceptor combining policy, capabilities, budget, and approval.
///
/// This is the single entry point for all security checks. All actions flow
/// through `intercept()` before execution.
pub struct SecurityInterceptor {
    capability_validator: CapabilityValidator,
    budget_validator: BudgetValidator,
    allowance_validator: AllowanceValidator,

    approval_manager: Arc<ApprovalManager>,
    policy: SecurityPolicy,
    audit_log: Arc<AuditLog>,
    session_id: SessionId,
    user_id: [u8; 8],
}

impl SecurityInterceptor {
    /// Create a new security interceptor.
    #[must_use]
    #[expect(clippy::too_many_arguments)]
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
            user_id: runtime_key.key_id(),
            capability_validator: CapabilityValidator::new(capability_store, runtime_key.clone()),
            budget_validator: BudgetValidator::new(budget_tracker, workspace_budget_tracker),
            allowance_validator: AllowanceValidator::new(
                allowance_store,
                runtime_key,
                workspace_root,
            ),
            approval_manager,
            policy,
            audit_log,
            session_id,
        }
    }

    /// Intercept an action and determine if it should proceed.
    ///
    /// This is the main entry point. Applies intersection semantics:
    /// policy, capability, budget, and approval checks in sequence.
    ///
    /// `principal` identifies the invoking agent — allowance and capability
    /// lookups are scoped to it (Layer 4, issue #668). Single-tenant callers
    /// pass `PrincipalId::default()`.
    ///
    /// # Errors
    ///
    /// Returns `ApprovalError` if the action is denied by policy, budget,
    /// or user decision.
    #[expect(clippy::too_many_lines)]
    pub async fn intercept(
        &self,
        principal: &PrincipalId,
        action: &SensitiveAction,
        context: &str,
        estimated_cost: Option<f64>,
    ) -> ApprovalResult<InterceptResult> {
        // Step 1: Policy check (hard boundaries)
        let policy_result = self.policy.check(action);
        if let PolicyResult::Blocked { reason } = &policy_result {
            self.audit_denied(action, reason)?;
            return Err(ApprovalError::PolicyBlocked {
                tool: action.action_type().to_string(),
                reason: reason.clone(),
            });
        }

        // Step 2: Capability check (scoped to the invoking principal)
        if let Some(proof) = self
            .capability_validator
            .check_capability(principal, action)
        {
            let mut cap_budget_warning = None;
            let mut reservation = None;
            if let Some(cost) = estimated_cost {
                match self.budget_validator.check_and_reserve(cost) {
                    Ok(res) => {
                        cap_budget_warning = res.warning().cloned();
                        reservation = Some(res);
                    },
                    Err(e) => {
                        self.audit_denied(action, &e.to_string())?;
                        return Err(e);
                    },
                }
            }
            let audit_id = self.audit_allowed(action, &proof)?;
            if let Some(res) = reservation {
                res.commit();
            }
            return Ok(InterceptResult {
                proof,
                audit_id,
                budget_warning: cap_budget_warning,
            });
        }

        // Step 3: Budget check (atomic check + reserve)
        let mut budget_warning = None;
        let mut budget_reservation = None;
        if let Some(cost) = estimated_cost {
            match self.budget_validator.check_and_reserve(cost) {
                Ok(res) => {
                    budget_warning = res.warning().cloned();
                    budget_reservation = Some(res);
                },
                Err(e) => {
                    self.audit_denied(action, &e.to_string())?;
                    return Err(e);
                },
            }
        }

        // Step 4: Risk assessment / Approval
        if matches!(policy_result, PolicyResult::Allowed) {
            let proof = InterceptProof::PolicyAllowed;
            let audit_id = self.audit_allowed(action, &proof)?;
            if let Some(res) = budget_reservation {
                res.commit();
            }
            return Ok(InterceptResult {
                proof,
                audit_id,
                budget_warning,
            });
        }

        // Policy requires approval (or action has inherent risk) — go to approval manager
        let outcome = self
            .approval_manager
            .check_approval(
                principal,
                action,
                context,
                self.allowance_validator.workspace_root.as_deref(),
            )
            .await;

        match outcome {
            ApprovalOutcome::Allowed { proof } => {
                if let Some(res) = budget_reservation {
                    res.commit();
                }
                let intercept_proof = match proof {
                    ApprovalProof::Allowance { allowance_id }
                    | ApprovalProof::CustomAllowance { allowance_id } => {
                        InterceptProof::Allowance { allowance_id }
                    },
                    ApprovalProof::OneTimeApproval => {
                        let audit_action = sensitive_action_to_audit(action);
                        let approval_audit_id = self
                            .audit_log
                            .append(
                                self.session_id.clone(),
                                audit_action,
                                AuditAuthProof::UserApproval {
                                    user_id: self.user_id,
                                    approval_entry_id: None,
                                },
                                AuditOutcome::success(),
                            )
                            .map_err(|e| ApprovalError::AuditFailed(e.to_string()))?;
                        return Ok(InterceptResult {
                            proof: InterceptProof::UserApproval {
                                approval_audit_id: approval_audit_id.clone(),
                            },
                            audit_id: approval_audit_id,
                            budget_warning,
                        });
                    },
                    ApprovalProof::SessionApproval { .. } => {
                        let audit_action = sensitive_action_to_audit(action);
                        let approval_audit_id = self
                            .audit_log
                            .append(
                                self.session_id.clone(),
                                audit_action,
                                AuditAuthProof::UserApproval {
                                    user_id: self.user_id,
                                    approval_entry_id: None,
                                },
                                AuditOutcome::success(),
                            )
                            .map_err(|e| ApprovalError::AuditFailed(e.to_string()))?;
                        let proof = self.allowance_validator.create_allowance_for_action(
                            principal,
                            action,
                            true,
                            approval_audit_id.clone(),
                        );
                        return Ok(InterceptResult {
                            proof,
                            audit_id: approval_audit_id,
                            budget_warning,
                        });
                    },
                    ApprovalProof::WorkspaceApproval { .. } => {
                        let audit_action = sensitive_action_to_audit(action);
                        let approval_audit_id = self
                            .audit_log
                            .append(
                                self.session_id.clone(),
                                audit_action,
                                AuditAuthProof::UserApproval {
                                    user_id: self.user_id,
                                    approval_entry_id: None,
                                },
                                AuditOutcome::success(),
                            )
                            .map_err(|e| ApprovalError::AuditFailed(e.to_string()))?;
                        let proof = self.allowance_validator.create_allowance_for_action(
                            principal,
                            action,
                            false,
                            approval_audit_id.clone(),
                        );
                        return Ok(InterceptResult {
                            proof,
                            audit_id: approval_audit_id,
                            budget_warning,
                        });
                    },
                    ApprovalProof::AlwaysAllow => {
                        let audit_action = sensitive_action_to_audit(action);
                        let approval_audit_id = self
                            .audit_log
                            .append(
                                self.session_id.clone(),
                                audit_action,
                                AuditAuthProof::UserApproval {
                                    user_id: self.user_id,
                                    approval_entry_id: None,
                                },
                                AuditOutcome::success(),
                            )
                            .map_err(|e| ApprovalError::AuditFailed(e.to_string()))?;

                        let result = self.capability_validator.handle_allow_always(
                            principal,
                            action,
                            approval_audit_id.clone(),
                        );
                        if let Ok(r) = result {
                            return Ok(InterceptResult {
                                proof: r,
                                audit_id: approval_audit_id,
                                budget_warning,
                            });
                        }
                        // Fall back to one-time approval if creation fails
                        let proof = InterceptProof::UserApproval {
                            approval_audit_id: approval_audit_id.clone(),
                        };
                        return Ok(InterceptResult {
                            proof,
                            audit_id: approval_audit_id,
                            budget_warning,
                        });
                    },
                };
                let audit_id = self.audit_allowed(action, &intercept_proof)?;
                Ok(InterceptResult {
                    proof: intercept_proof,
                    audit_id,
                    budget_warning,
                })
            },
            ApprovalOutcome::Denied { reason } => {
                self.audit_denied(action, &reason)?;
                Err(ApprovalError::Denied { reason })
            },
            ApprovalOutcome::Deferred {
                resolution_id,
                fallback,
            } => {
                let reason =
                    format!("action deferred (resolution: {resolution_id}, fallback: {fallback})");
                self.audit_deferred(action, &reason)?;
                Err(ApprovalError::Deferred)
            },
        }
    }

    /// Log an allowed action to the audit trail (fail-closed).
    ///
    /// # Errors
    ///
    /// Returns `ApprovalError::AuditFailed` if the audit entry cannot be
    /// written. The caller must not proceed with the action.
    fn audit_allowed(
        &self,
        action: &SensitiveAction,
        proof: &InterceptProof,
    ) -> ApprovalResult<AuditEntryId> {
        let audit_action = sensitive_action_to_audit(action);
        let auth_proof = intercept_proof_to_audit(proof, self.user_id);

        self.audit_log
            .append(
                self.session_id.clone(),
                audit_action,
                auth_proof,
                AuditOutcome::success(),
            )
            .map_err(|e| ApprovalError::AuditFailed(e.to_string()))
    }

    /// Log a denied action to the audit trail (fail-closed).
    ///
    /// # Errors
    ///
    /// Returns `ApprovalError::AuditFailed` if the audit entry cannot be
    /// written.
    fn audit_denied(&self, action: &SensitiveAction, reason: &str) -> ApprovalResult<()> {
        let audit_action = sensitive_action_to_audit(action);
        self.audit_log
            .append(
                self.session_id.clone(),
                audit_action,
                AuditAuthProof::Denied {
                    reason: reason.to_string(),
                },
                AuditOutcome::failure(reason),
            )
            .map(|_| ())
            .map_err(|e| ApprovalError::AuditFailed(e.to_string()))
    }

    /// Log a deferred action to the audit trail (fail-closed).
    ///
    /// # Errors
    ///
    /// Returns `ApprovalError::AuditFailed` if the audit entry cannot be
    /// written.
    fn audit_deferred(&self, action: &SensitiveAction, reason: &str) -> ApprovalResult<()> {
        let audit_action = sensitive_action_to_audit(action);
        self.audit_log
            .append(
                self.session_id.clone(),
                audit_action,
                AuditAuthProof::Denied {
                    reason: reason.to_string(),
                },
                AuditOutcome::failure(reason),
            )
            .map(|_| ())
            .map_err(|e| ApprovalError::AuditFailed(e.to_string()))
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
        &self.budget_validator.tracker
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

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
