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
    budget_tracker: Arc<BudgetTracker>,
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
        Arc::clone(&budget_tracker),
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
        budget_tracker,
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
    let result = interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
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
    let result = interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
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

    let result = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
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

    let result = interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
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
    #[expect(clippy::float_cmp)]
    {
        assert_eq!(interceptor.budget_tracker().spent(), 0.0);
    }

    // Pass a cost of 5.0. It should be reserved, but then refunded when denied.
    let result = interceptor
        .intercept(&PrincipalId::default(), &action, "test", Some(5.0))
        .await;
    assert!(result.is_err());

    // Assert budget spent is back to 0
    #[expect(clippy::float_cmp)]
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
        async fn request_approval(&self, _request: ApprovalRequest) -> Option<ApprovalResponse> {
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
    #[expect(clippy::float_cmp)]
    {
        assert_eq!(interceptor.budget_tracker().spent(), 0.0);
    }

    // Start intercept task
    let principal = PrincipalId::default();
    let fut = interceptor.intercept(&principal, &action, "test", Some(5.0));

    // Let it run for a moment so it hits the pending await point and reserves budget
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), fut).await;

    // The timeout drops the future, which drops the budget reservation guard.
    // Assert budget spent is back to 0
    #[expect(clippy::float_cmp)]
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
    let result = interceptor
        .intercept(&PrincipalId::default(), &action, "test", Some(15.0))
        .await;

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

    let result = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", Some(15.0))
        .await;

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
async fn test_budget_committed_on_approval() {
    let t = make_interceptor_with_audit(
        SecurityPolicy::default(),
        Some(Arc::new(SessionApproveHandler)),
    )
    .await;

    let action = SensitiveAction::McpToolCall {
        server: "test".to_string(),
        tool: "expensive_read".to_string(),
    };

    // Call intercept with a cost. SessionApproveHandler will approve it.
    let result = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", Some(5.0))
        .await;
    assert!(result.is_ok(), "Expected action to be approved");

    // Verify the budget was actually committed, not refunded
    let snapshot = t.budget_tracker.snapshot();
    assert!(
        (snapshot.session_spent_usd - 5.0).abs() < f64::EPSILON,
        "Expected budget to be committed, but it was refunded"
    );
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
    let result1 = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", Some(5.0))
        .await;
    assert!(result1.is_ok());

    // Second call — the capability exists, but now the cost exceeds the per-action limit (15.0 > 10.0).
    let result2 = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", Some(15.0))
        .await;
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
    #[expect(clippy::float_cmp)]
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

    let result = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
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

    let result = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
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
    let result1 = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
    assert!(result1.is_ok());

    let count_after_first = t.audit_log.count_session(&t.session_id).unwrap();
    assert_eq!(
        count_after_first, 1,
        "first session approval should create exactly one audit entry"
    );

    // Second call for same action — allowance should match, creating
    // another audit entry for the allowance-based authorization
    let result2 = t
        .interceptor
        .intercept(&PrincipalId::default(), &action, "test", None)
        .await;
    assert!(result2.is_ok());

    let count_after_second = t.audit_log.count_session(&t.session_id).unwrap();
    assert_eq!(
        count_after_second, 2,
        "second call should add one more audit entry (allowance-based)"
    );
}
