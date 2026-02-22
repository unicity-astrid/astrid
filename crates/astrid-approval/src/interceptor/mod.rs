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
    user_id: [u8; 8],
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
            let mut reservation = None;
            if let Some(cost) = estimated_cost {
                match self.budget_validator.check_and_reserve(cost) {
                    Ok(res) => {
                        cap_budget_warning = res.warning().cloned();
                        reservation = Some(res);
                    },
                    Err(e) => {
                        self.audit_denied(action, &e.to_string());
                        return Err(e);
                    },
                }
            }
            let audit_id = self.audit_allowed(action, &proof);
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
                    self.audit_denied(action, &e.to_string());
                    return Err(e);
                },
            }
        }

        // Step 4: Risk assessment / Approval
        if matches!(policy_result, PolicyResult::Allowed) {
            let proof = InterceptProof::PolicyAllowed;
            let audit_id = self.audit_allowed(action, &proof);
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
                            .unwrap_or_default();
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
                            .unwrap_or_default();
                        let proof = self.allowance_validator.create_allowance_for_action(
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
                            .unwrap_or_default();
                        let proof = self.allowance_validator.create_allowance_for_action(
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
                            .unwrap_or_default();

                        let result = self
                            .capability_validator
                            .handle_allow_always(action, approval_audit_id.clone());
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
                let audit_id = self.audit_allowed(action, &intercept_proof);
                if let Some(res) = budget_reservation {
                    res.commit();
                }
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
                Err(ApprovalError::Deferred)
            },
        }
    }

    /// Log an allowed action to the audit trail.
    fn audit_allowed(&self, action: &SensitiveAction, proof: &InterceptProof) -> AuditEntryId {
        let audit_action = sensitive_action_to_audit(action);
        let auth_proof = intercept_proof_to_audit(proof, self.user_id);

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
            AuditOutcome::failure(reason),
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

    /// Auto-approve handler for tests (one-time approval).
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

    /// Session-scoped approval handler for tests.
    struct SessionApproveHandler;

    #[async_trait::async_trait]
    impl ApprovalHandler for SessionApproveHandler {
        async fn request_approval(&self, request: ApprovalRequest) -> Option<ApprovalResponse> {
            Some(ApprovalResponse::new(
                request.id,
                ApprovalDecision::ApproveSession,
            ))
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    /// Workspace-scoped approval handler for tests.
    struct WorkspaceApproveHandler;

    #[async_trait::async_trait]
    impl ApprovalHandler for WorkspaceApproveHandler {
        async fn request_approval(&self, request: ApprovalRequest) -> Option<ApprovalResponse> {
            Some(ApprovalResponse::new(
                request.id,
                ApprovalDecision::ApproveWorkspace,
            ))
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    /// Build result holding the interceptor plus shared handles for test assertions.
    struct TestInterceptor {
        interceptor: SecurityInterceptor,
        audit_log: Arc<AuditLog>,
        session_id: SessionId,
    }

    async fn make_interceptor_with_audit(
        policy: SecurityPolicy,
        handler: Option<Arc<dyn ApprovalHandler>>,
    ) -> TestInterceptor {
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
            Arc::clone(&audit_log),
            runtime_key,
            session_id.clone(),
            allowance_store,
            None,
            None,
        );

        if let Some(h) = handler {
            interceptor.approval_manager.register_handler(h).await;
        }

        TestInterceptor {
            interceptor,
            audit_log,
            session_id,
        }
    }

    async fn make_interceptor(
        policy: SecurityPolicy,
        handler: Option<Arc<dyn ApprovalHandler>>,
    ) -> SecurityInterceptor {
        make_interceptor_with_audit(policy, handler)
            .await
            .interceptor
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
        let err = result.expect_err("should be blocked by policy");
        assert!(
            matches!(err, ApprovalError::PolicyBlocked { .. }),
            "expected PolicyBlocked, got {err:?}"
        );
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
        let t = make_interceptor_with_audit(SecurityPolicy::default(), Some(handler)).await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };

        let result = t.interceptor.intercept(&action, "test", None).await;
        assert!(result.is_ok());

        let ok = result.unwrap();
        // AutoApproveHandler gives OneTimeApproval — creates exactly one audit entry
        assert!(matches!(ok.proof, InterceptProof::UserApproval { .. }));

        let count = t.audit_log.count_session(&t.session_id).unwrap();
        assert_eq!(
            count, 1,
            "one-time approval should create exactly one audit entry"
        );

        let entries = t.audit_log.get_session_entries(&t.session_id).unwrap();
        let entry = entries.first().unwrap();
        match &entry.authorization {
            astrid_audit::AuthorizationProof::UserApproval { user_id, .. } => {
                assert_eq!(user_id, &t.interceptor.user_id);
            },
            _ => panic!("Expected UserApproval authorization proof"),
        }
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

    #[tokio::test]
    async fn test_budget_refunded_on_denial() {
        let handler = Arc::new(AutoDenyHandler);
        let interceptor = make_interceptor(SecurityPolicy::default(), Some(handler)).await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };

        // Assert budget spent is 0
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(interceptor.budget_tracker().spent(), 0.0);
        }

        // Pass a cost of 5.0. It should be reserved, but then refunded when denied.
        let result = interceptor.intercept(&action, "test", Some(5.0)).await;
        assert!(result.is_err());

        // Assert budget spent is back to 0
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(interceptor.budget_tracker().spent(), 0.0);
        }
    }

    #[tokio::test]
    async fn test_budget_refunded_on_async_cancellation() {
        // A handler that never returns, so we can cancel the future
        struct HangingHandler;
        #[async_trait::async_trait]
        impl ApprovalHandler for HangingHandler {
            async fn request_approval(
                &self,
                _request: ApprovalRequest,
            ) -> Option<ApprovalResponse> {
                std::future::pending().await
            }
            fn is_available(&self) -> bool {
                true
            }
        }

        let handler = Arc::new(HangingHandler);
        let interceptor = make_interceptor(SecurityPolicy::default(), Some(handler)).await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };

        // Assert budget spent is 0
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(interceptor.budget_tracker().spent(), 0.0);
        }

        // Start intercept task
        let fut = interceptor.intercept(&action, "test", Some(5.0));

        // Let it run for a moment so it hits the pending await point and reserves budget
        let _ = tokio::time::timeout(std::time::Duration::from_millis(50), fut).await;

        // The timeout drops the future, which drops the budget reservation guard.
        // Assert budget spent is back to 0
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(interceptor.budget_tracker().spent(), 0.0);
        }
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

    #[tokio::test]
    async fn test_budget_exceeded_creates_audit_entry() {
        let handler = Arc::new(AutoApproveHandler);
        let t = make_interceptor_with_audit(SecurityPolicy::default(), Some(handler)).await;

        let action = SensitiveAction::McpToolCall {
            server: "financial".to_string(),
            tool: "transfer".to_string(),
        };

        let result = t.interceptor.intercept(&action, "test", Some(15.0)).await;

        assert!(result.is_err());

        let count = t.audit_log.count_session(&t.session_id).unwrap();
        assert_eq!(
            count, 1,
            "budget denied action should create exactly one audit entry"
        );

        let entries = t.audit_log.get_session_entries(&t.session_id).unwrap();
        let entry = entries.first().unwrap();
        match &entry.authorization {
            astrid_audit::AuthorizationProof::Denied { reason } => {
                assert!(reason.contains("budget exceeded"));
            },
            _ => panic!("Expected Denied authorization proof"),
        }
    }

    #[tokio::test]
    async fn test_capability_budget_exceeded_creates_audit_entry() {
        let t = make_interceptor_with_audit(
            SecurityPolicy::default(),
            Some(Arc::new(SessionApproveHandler)),
        )
        .await;

        let action = SensitiveAction::McpToolCall {
            server: "test".to_string(),
            tool: "expensive_read".to_string(),
        };

        // First call — establishes the capability (allowance) for the session.
        // The cost is 5.0, which is well within the 10.0 per-action limit.
        let result1 = t.interceptor.intercept(&action, "test", Some(5.0)).await;
        assert!(result1.is_ok());

        // Second call — the capability exists, but now the cost exceeds the per-action limit (15.0 > 10.0).
        let result2 = t.interceptor.intercept(&action, "test", Some(15.0)).await;
        assert!(result2.is_err());

        // There should be 2 audit entries:
        // 1. The initial session approval
        // 2. The budget denial on the second attempt
        let count = t.audit_log.count_session(&t.session_id).unwrap();
        assert_eq!(
            count, 2,
            "expected two audit entries: initial approval, followed by budget denial"
        );

        let entries = t.audit_log.get_session_entries(&t.session_id).unwrap();
        let last_entry = entries.last().unwrap();
        match &last_entry.authorization {
            astrid_audit::AuthorizationProof::Denied { reason } => {
                assert!(reason.contains("budget exceeded"));
            },
            _ => panic!("Expected Denied authorization proof for the second call"),
        }
    }

    #[tokio::test]
    async fn test_budget_rollback_on_dual_budget_denial() {
        // Workspace budget is large, session budget is small.
        let ws_tracker = Arc::new(WorkspaceBudgetTracker::new(Some(100.0), 80));
        let session_tracker = Arc::new(BudgetTracker::new(BudgetConfig::new(10.0, 50.0)));
        let budget_validator = BudgetValidator::new(session_tracker, Some(ws_tracker.clone()));

        // Cost is 50. This is fine for workspace (limit 100), but exceeds session limit (10).
        // It's also within per_action limit of session_tracker (50).
        let cost = 50.0;

        let result = budget_validator.check_and_reserve(cost);

        // Should fail because of session budget.
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("budget exceeded (session budget)"));

        // Critically, the workspace budget should STILL BE 100.0 (not deducted).
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(ws_tracker.spent(), 0.0);
            assert_eq!(ws_tracker.remaining(), Some(100.0));
        }
    }

    // -----------------------------------------------------------------------
    // Session approval — creates audit entry and allowance
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_session_approval_creates_audit_entry() {
        let t = make_interceptor_with_audit(
            SecurityPolicy::default(),
            Some(Arc::new(SessionApproveHandler)),
        )
        .await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };

        let result = t.interceptor.intercept(&action, "test", None).await;
        assert!(result.is_ok());

        let ok = result.unwrap();
        assert!(
            matches!(ok.proof, InterceptProof::SessionApproval { .. }),
            "expected SessionApproval proof, got {:?}",
            ok.proof
        );

        // Exactly one audit entry should exist for this session
        let count = t.audit_log.count_session(&t.session_id).unwrap();
        assert_eq!(
            count, 1,
            "session approval should create exactly one audit entry"
        );
    }

    // -----------------------------------------------------------------------
    // Workspace approval — creates audit entry and allowance
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_workspace_approval_creates_audit_entry() {
        let t = make_interceptor_with_audit(
            SecurityPolicy::default(),
            Some(Arc::new(WorkspaceApproveHandler)),
        )
        .await;

        let action = SensitiveAction::FileDelete {
            path: "/home/user/file.txt".to_string(),
        };

        let result = t.interceptor.intercept(&action, "test", None).await;
        assert!(result.is_ok());

        let ok = result.unwrap();
        assert!(
            matches!(ok.proof, InterceptProof::WorkspaceApproval { .. }),
            "expected WorkspaceApproval proof, got {:?}",
            ok.proof
        );

        // Exactly one audit entry should exist for this session
        let count = t.audit_log.count_session(&t.session_id).unwrap();
        assert_eq!(
            count, 1,
            "workspace approval should create exactly one audit entry"
        );
    }

    // -----------------------------------------------------------------------
    // Session approval — no duplicate audit entries
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_session_approval_no_duplicate_audit_entry() {
        let t = make_interceptor_with_audit(
            SecurityPolicy::default(),
            Some(Arc::new(SessionApproveHandler)),
        )
        .await;

        let action = SensitiveAction::McpToolCall {
            server: "test".to_string(),
            tool: "read".to_string(),
        };

        // First call — should create one audit entry
        let result1 = t.interceptor.intercept(&action, "test", None).await;
        assert!(result1.is_ok());

        let count_after_first = t.audit_log.count_session(&t.session_id).unwrap();
        assert_eq!(
            count_after_first, 1,
            "first session approval should create exactly one audit entry"
        );

        // Second call for same action — allowance should match, creating
        // another audit entry for the allowance-based authorization
        let result2 = t.interceptor.intercept(&action, "test", None).await;
        assert!(result2.is_ok());

        let count_after_second = t.audit_log.count_session(&t.session_id).unwrap();
        assert_eq!(
            count_after_second, 2,
            "second call should add one more audit entry (allowance-based)"
        );
    }
}
