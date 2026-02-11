//! Integration tests for budget enforcement.

use std::sync::Arc;

use astralis_approval::deferred::DeferredResolutionStore;
use astralis_approval::manager::{ApprovalHandler, ApprovalManager};
use astralis_approval::request::{
    ApprovalDecision as InternalDecision, ApprovalRequest as InternalRequest,
    ApprovalResponse as InternalResponse,
};
use astralis_approval::{
    AllowanceStore, BudgetConfig, BudgetResult, BudgetTracker, SecurityInterceptor, SecurityPolicy,
    SensitiveAction,
};
use astralis_audit::AuditLog;
use astralis_capabilities::CapabilityStore;
use astralis_core::SessionId;
use astralis_crypto::KeyPair;

/// Auto-approve handler for tests.
struct AutoApproveHandler;

#[async_trait::async_trait]
impl ApprovalHandler for AutoApproveHandler {
    async fn request_approval(&self, request: InternalRequest) -> Option<InternalResponse> {
        Some(InternalResponse::new(request.id, InternalDecision::Approve))
    }
    fn is_available(&self) -> bool {
        true
    }
}

async fn make_interceptor_with_budget(
    budget_config: BudgetConfig,
) -> (SecurityInterceptor, Arc<BudgetTracker>) {
    let capability_store = Arc::new(CapabilityStore::in_memory());
    let allowance_store = Arc::new(AllowanceStore::new());
    let deferred_queue = Arc::new(DeferredResolutionStore::new());
    let approval_manager = Arc::new(ApprovalManager::new(
        Arc::clone(&allowance_store),
        deferred_queue,
    ));
    let budget_tracker = Arc::new(BudgetTracker::new(budget_config));
    let audit_log = Arc::new(AuditLog::in_memory(KeyPair::generate()));
    let runtime_key = Arc::new(KeyPair::generate());
    let session_id = SessionId::new();

    approval_manager
        .register_handler(Arc::new(AutoApproveHandler) as Arc<dyn ApprovalHandler>)
        .await;

    let interceptor = SecurityInterceptor::new(
        capability_store,
        approval_manager,
        SecurityPolicy::default(),
        Arc::clone(&budget_tracker),
        audit_log,
        runtime_key,
        session_id,
        allowance_store,
        None,
        None,
    );

    (interceptor, budget_tracker)
}

#[tokio::test]
async fn test_budget_exceeded_blocks_action() {
    // Very low budget: $0.01 session max, $0.01 per action
    let (interceptor, tracker) = make_interceptor_with_budget(BudgetConfig::new(0.01, 0.01)).await;

    // Record cost to nearly deplete budget
    tracker.record_cost(0.009);

    // Now try an action with cost exceeding remaining
    let action = SensitiveAction::NetworkRequest {
        host: "api.example.com".to_string(),
        port: 443,
    };

    let result = interceptor
        .intercept(&action, "expensive call", Some(0.05))
        .await;
    assert!(result.is_err(), "should be denied due to budget");
    assert!(
        result.unwrap_err().to_string().contains("budget"),
        "error should mention budget"
    );
}

#[tokio::test]
async fn test_budget_per_action_limit_exceeded() {
    // Per-action limit of $5, session limit of $100
    let (interceptor, _tracker) = make_interceptor_with_budget(BudgetConfig::new(100.0, 5.0)).await;

    let action = SensitiveAction::NetworkRequest {
        host: "api.example.com".to_string(),
        port: 443,
    };

    // Try to spend $10 in one action (exceeds $5 per-action limit)
    let result = interceptor.intercept(&action, "big call", Some(10.0)).await;
    assert!(result.is_err(), "should be denied due to per-action limit");
}

#[tokio::test]
async fn test_budget_warning_threshold() {
    // Session budget $1.00, warn at 80%
    let config = BudgetConfig::new(1.0, 1.0).with_warn_at_percent(80);
    let tracker = BudgetTracker::new(config);

    // Record cost past 80% threshold
    tracker.record_cost(0.85);

    // Check budget for a small additional cost
    let result = tracker.check_budget(0.05);
    assert!(
        matches!(result, BudgetResult::WarnAndAllow { .. }),
        "should warn when past threshold, got: {result:?}"
    );
    assert!(result.is_allowed(), "warn should still allow");
}

#[tokio::test]
async fn test_budget_warning_surfaced_in_intercept() {
    // Session budget $1.00, warn at 80%
    let config = BudgetConfig::new(1.0, 1.0).with_warn_at_percent(80);
    let (interceptor, tracker) = make_interceptor_with_budget(config).await;

    // Record cost past 80% threshold
    tracker.record_cost(0.85);

    let action = SensitiveAction::NetworkRequest {
        host: "api.example.com".to_string(),
        port: 443,
    };

    // Intercept should succeed but include a budget warning
    let result = interceptor
        .intercept(&action, "call near budget", Some(0.05))
        .await;
    assert!(result.is_ok(), "action should still be allowed");
    let intercept_result = result.unwrap();
    assert!(
        intercept_result.budget_warning.is_some(),
        "should have budget warning"
    );
    let warning = intercept_result.budget_warning.unwrap();
    assert!(warning.percent_used >= 80.0, "should be at or above 80%");
}

#[tokio::test]
async fn test_budget_within_limits_allowed() {
    let (interceptor, _tracker) =
        make_interceptor_with_budget(BudgetConfig::new(100.0, 10.0)).await;

    let action = SensitiveAction::NetworkRequest {
        host: "api.example.com".to_string(),
        port: 443,
    };

    let result = interceptor
        .intercept(&action, "cheap call", Some(5.0))
        .await;
    assert!(result.is_ok(), "should be allowed within budget");
    assert!(
        result.unwrap().budget_warning.is_none(),
        "should not have budget warning when well within limits"
    );
}

#[tokio::test]
async fn test_budget_tracking_accumulates() {
    let tracker = BudgetTracker::new(BudgetConfig::new(10.0, 5.0));

    assert_eq!(tracker.spent(), 0.0);
    assert_eq!(tracker.remaining(), 10.0);

    tracker.record_cost(3.0);
    assert!((tracker.spent() - 3.0).abs() < f64::EPSILON);
    assert!((tracker.remaining() - 7.0).abs() < f64::EPSILON);

    tracker.record_cost(4.0);
    assert!((tracker.spent() - 7.0).abs() < f64::EPSILON);
    assert!((tracker.remaining() - 3.0).abs() < f64::EPSILON);

    // Check that a cost exceeding remaining is blocked
    let result = tracker.check_budget(5.0);
    assert!(result.is_exceeded());
}
