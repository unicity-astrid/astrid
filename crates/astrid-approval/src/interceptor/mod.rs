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

pub use allowance::AllowanceValidator;
pub use budget::BudgetValidator;
pub use capability::CapabilityValidator;
pub use types::*;

use crate::error::{ApprovalError, ApprovalResult};
use astrid_audit::{AuditEntryId, AuditLog, AuditOutcome, AuthorizationProof as AuditAuthProof};
use astrid_capabilities::CapabilityStore;
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
        if let Some(proof) = self.capability_validator.check_capability(action) {
            let mut cap_budget_warning = None;
            if let Some(cost) = estimated_cost {
                cap_budget_warning = self.budget_validator.check_workspace_budget(cost)?;
                if let Some(warning) = self.budget_validator.check_session_budget(cost)? {
                    cap_budget_warning = Some(warning);
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
            budget_warning = self.budget_validator.check_workspace_budget(cost)?;
            if let Some(warning) = self.budget_validator.check_session_budget(cost)? {
                budget_warning = Some(warning);
            }
        }

        // Step 4: Risk assessment / Approval
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
            .check_approval(
                action,
                context,
                self.allowance_validator.workspace_root.as_deref(),
            )
            .await;

        match outcome {
            ApprovalOutcome::Allowed { proof } => {
                let intercept_proof = match proof {
                    ApprovalProof::Allowance { allowance_id }
                    | ApprovalProof::CustomAllowance { allowance_id } => {
                        InterceptProof::Allowance { allowance_id }
                    },
                    ApprovalProof::OneTimeApproval => InterceptProof::UserApproval {
                        approval_audit_id: AuditEntryId::new(),
                    },
                    ApprovalProof::SessionApproval { .. } => self
                        .allowance_validator
                        .create_allowance_for_action(action, true),
                    ApprovalProof::WorkspaceApproval { .. } => self
                        .allowance_validator
                        .create_allowance_for_action(action, false),
                    ApprovalProof::AlwaysAllow => {
                        let audit_action = sensitive_action_to_audit(action);
                        let approval_audit_id = self
                            .audit_log
                            .append(
                                self.session_id.clone(),
                                audit_action,
                                AuditAuthProof::UserApproval {
                                    user_id: self.capability_validator.runtime_key.key_id(),
                                    approval_entry_id: AuditEntryId::new(),
                                },
                                AuditOutcome::success(),
                            )
                            .unwrap_or_default();

                        let result = self
                            .capability_validator
                            .handle_allow_always(action, approval_audit_id.clone());
                        if let Ok(r) = result {
                            let audit_id = self.audit_allowed(action, &r);
                            return Ok(InterceptResult {
                                proof: r,
                                audit_id,
                                budget_warning,
                            });
                        }
                        // Fall back to one-time approval if creation fails
                        let proof = InterceptProof::UserApproval { approval_audit_id };
                        let audit_id = self.audit_allowed(action, &proof);
                        return Ok(InterceptResult {
                            proof,
                            audit_id,
                            budget_warning,
                        });
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
        let handler = Arc::new(AutoApproveHandler);
        let interceptor = make_interceptor(SecurityPolicy::default(), Some(handler)).await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };

        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_ok());

        let ok = result.unwrap();
        // AutoApproveHandler gives OneTimeApproval by default since it intercepts before allowance UI
        assert!(matches!(ok.proof, InterceptProof::UserApproval { .. }));
    }

    // -----------------------------------------------------------------------
    // Requires approval — denied
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_requires_approval_denied() {
        let handler = Arc::new(AutoDenyHandler);
        let interceptor = make_interceptor(SecurityPolicy::default(), Some(handler)).await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };

        let result = interceptor.intercept(&action, "test", None).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Budget exceeded
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_budget_exceeded() {
        let handler = Arc::new(AutoApproveHandler);
        let interceptor = make_interceptor(SecurityPolicy::default(), Some(handler)).await;

        let action = SensitiveAction::McpToolCall {
            server: "financial".to_string(),
            tool: "transfer".to_string(),
        };

        // max_per_action is 10.0, session_max is 100.0 (from `make_interceptor`)
        let result = interceptor.intercept(&action, "test", Some(15.0)).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("budget exceeded"));
    }
}
