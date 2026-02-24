//! Integration tests for capability token lifecycle.
//!
//! Tests the "Allow Always" flow where approving an action creates a persistent
//! capability token that auto-approves future identical actions.

use std::sync::Arc;

use astrid_approval::deferred::DeferredResolutionStore;
use astrid_approval::manager::{ApprovalHandler, ApprovalManager};
use astrid_approval::request::{
    ApprovalDecision as InternalDecision, ApprovalRequest as InternalRequest,
    ApprovalResponse as InternalResponse,
};
use astrid_approval::{
    AllowanceStore, BudgetConfig, BudgetTracker, SecurityInterceptor, SecurityPolicy,
    SensitiveAction,
};
use astrid_audit::AuditLog;
use astrid_capabilities::CapabilityStore;
use astrid_core::SessionId;
use astrid_crypto::KeyPair;

/// Handler that returns "Allow Always" for all requests.
struct AlwaysAlwaysHandler;

#[async_trait::async_trait]
impl ApprovalHandler for AlwaysAlwaysHandler {
    async fn request_approval(&self, request: InternalRequest) -> Option<InternalResponse> {
        Some(InternalResponse::new(
            request.id,
            InternalDecision::ApproveAlways,
        ))
    }
    fn is_available(&self) -> bool {
        true
    }
}

/// Handler that auto-denies everything.
struct AutoDenyHandler;

#[async_trait::async_trait]
impl ApprovalHandler for AutoDenyHandler {
    async fn request_approval(&self, request: InternalRequest) -> Option<InternalResponse> {
        Some(InternalResponse::new(
            request.id,
            InternalDecision::Deny {
                reason: "test deny".to_string(),
            },
        ))
    }
    fn is_available(&self) -> bool {
        true
    }
}

async fn make_interceptor(
    handler: Arc<dyn ApprovalHandler>,
) -> (SecurityInterceptor, Arc<CapabilityStore>) {
    let capability_store = Arc::new(CapabilityStore::in_memory());
    let allowance_store = Arc::new(AllowanceStore::new());
    let deferred_queue = Arc::new(DeferredResolutionStore::new());
    let approval_manager = Arc::new(ApprovalManager::new(
        Arc::clone(&allowance_store),
        deferred_queue,
    ));
    let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::new(100.0, 10.0)));
    let audit_log = Arc::new(AuditLog::in_memory(KeyPair::generate()));
    let runtime_key = Arc::new(KeyPair::generate());
    let session_id = SessionId::new();

    approval_manager.register_handler(handler).await;

    let interceptor = SecurityInterceptor::new(
        Arc::clone(&capability_store),
        approval_manager,
        SecurityPolicy::default(),
        budget_tracker,
        audit_log,
        runtime_key,
        session_id,
        allowance_store,
        None,
        None,
    );

    (interceptor, capability_store)
}

#[tokio::test]
async fn test_allow_always_creates_capability_and_second_call_uses_it() {
    let (interceptor, capability_store) = make_interceptor(Arc::new(AlwaysAlwaysHandler)).await;

    // First call: should be approved and create a capability token
    let action = SensitiveAction::FileWriteOutsideSandbox {
        path: "/home/user/test.txt".to_string(),
    };
    let result1 = interceptor.intercept(&action, "writing file", None).await;
    assert!(result1.is_ok(), "first call should succeed");

    let proof1 = result1.unwrap();
    assert!(
        matches!(
            proof1.proof,
            astrid_approval::InterceptProof::CapabilityCreated { .. }
        ),
        "should have created a capability"
    );

    // Verify the capability is in the store
    let found = capability_store.find_capability(
        "file:///home/user/test.txt",
        astrid_core::types::Permission::Write,
    );
    assert!(found.is_some(), "capability token should be in store");

    // Second call: same action — should be authorized by existing capability (no approval needed)
    // To prove this works even with a deny handler, we'll switch handlers.
    // But the interceptor already has AlwaysAlwaysHandler... the capability check happens
    // before the approval check, so it won't hit the handler.
    let result2 = interceptor.intercept(&action, "writing again", None).await;
    assert!(result2.is_ok(), "second call should succeed via capability");

    let proof2 = result2.unwrap();
    assert!(
        matches!(
            proof2.proof,
            astrid_approval::InterceptProof::Capability { .. }
        ),
        "second call should use existing capability, got: {:?}",
        proof2.proof
    );
}

#[tokio::test]
async fn test_capability_survives_across_interceptor_instances() {
    // Create first interceptor with "Allow Always"
    let capability_store = Arc::new(CapabilityStore::in_memory());
    let allowance_store = Arc::new(AllowanceStore::new());
    let deferred_queue = Arc::new(DeferredResolutionStore::new());
    let approval_manager = Arc::new(ApprovalManager::new(
        Arc::clone(&allowance_store),
        Arc::clone(&deferred_queue),
    ));
    let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::new(100.0, 10.0)));
    let audit_log = Arc::new(AuditLog::in_memory(KeyPair::generate()));
    let runtime_key = Arc::new(KeyPair::generate());
    let session_id = SessionId::new();

    approval_manager
        .register_handler(Arc::new(AlwaysAlwaysHandler) as Arc<dyn ApprovalHandler>)
        .await;

    let interceptor1 = SecurityInterceptor::new(
        Arc::clone(&capability_store),
        Arc::clone(&approval_manager),
        SecurityPolicy::default(),
        Arc::clone(&budget_tracker),
        Arc::clone(&audit_log),
        Arc::clone(&runtime_key),
        session_id.clone(),
        Arc::clone(&allowance_store),
        None,
        None,
    );

    let action = SensitiveAction::FileRead {
        path: "/workspace/data.txt".to_string(),
    };

    // First call creates capability
    let result1 = interceptor1.intercept(&action, "reading data", None).await;
    assert!(result1.is_ok());

    // Create a SECOND interceptor sharing the same capability store but with a DENY handler
    let allowance_store2 = Arc::new(AllowanceStore::new());
    let deferred_queue2 = Arc::new(DeferredResolutionStore::new());
    let approval_manager2 = Arc::new(ApprovalManager::new(
        Arc::clone(&allowance_store2),
        deferred_queue2,
    ));
    approval_manager2
        .register_handler(Arc::new(AutoDenyHandler) as Arc<dyn ApprovalHandler>)
        .await;

    let interceptor2 = SecurityInterceptor::new(
        Arc::clone(&capability_store), // Shared!
        approval_manager2,
        SecurityPolicy::default(),
        Arc::clone(&budget_tracker),
        Arc::clone(&audit_log),
        Arc::clone(&runtime_key),
        session_id,
        allowance_store2,
        None,
        None,
    );

    // Second interceptor should still find the capability from the first
    let result2 = interceptor2.intercept(&action, "reading again", None).await;
    assert!(
        result2.is_ok(),
        "should succeed via shared capability store despite deny handler"
    );
    assert!(matches!(
        result2.unwrap().proof,
        astrid_approval::InterceptProof::Capability { .. }
    ));
}