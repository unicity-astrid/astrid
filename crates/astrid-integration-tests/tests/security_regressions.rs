//! Regression tests for Step 8 security fixes.
//!
//! These tests verify fixes for 7 confirmed issues:
//! - Race condition in allowance find+consume (now atomic)
//! - Race condition in budget check+reserve (now atomic)
//! - Workspace budget bypass via capability-authorized actions
//! - Missing audit trail for session/workspace allowance creation
//! - String-splitting path traversal check (now uses Path::components)
//! - Expired allowance cleanup during atomic lookup
//! - Race condition in workspace budget check+reserve (now atomic)

use std::sync::Arc;

use astrid_approval::deferred::DeferredResolutionStore;
use astrid_approval::manager::{ApprovalHandler, ApprovalManager};
use astrid_approval::request::{
    ApprovalDecision as InternalDecision, ApprovalRequest as InternalRequest,
    ApprovalResponse as InternalResponse,
};
use astrid_approval::{
    Allowance, AllowanceId, AllowancePattern, AllowanceStore, BudgetConfig, BudgetTracker,
    SecurityInterceptor, SecurityPolicy, SensitiveAction, WorkspaceBudgetTracker,
};
use astrid_audit::AuditLog;
use astrid_capabilities::{CapabilityStore, CapabilityToken, ResourcePattern, TokenScope};
use astrid_core::SessionId;
use astrid_core::types::{Permission, Timestamp};
use astrid_crypto::KeyPair;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

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

/// Handler that returns "Approve Session" for all requests.
struct SessionApproveHandler;

#[async_trait::async_trait]
impl ApprovalHandler for SessionApproveHandler {
    async fn request_approval(&self, request: InternalRequest) -> Option<InternalResponse> {
        Some(InternalResponse::new(
            request.id,
            InternalDecision::ApproveSession,
        ))
    }
    fn is_available(&self) -> bool {
        true
    }
}

fn make_test_allowance(
    pattern: AllowancePattern,
    max_uses: Option<u32>,
    expires_at: Option<Timestamp>,
) -> Allowance {
    let keypair = KeyPair::generate();
    Allowance {
        id: AllowanceId::new(),
        action_pattern: pattern,
        created_at: Timestamp::now(),
        expires_at,
        max_uses,
        uses_remaining: max_uses,
        session_only: true,
        workspace_root: None,
        signature: keypair.sign(b"test-allowance"),
    }
}

// ---------------------------------------------------------------------------
// 1. Atomic allowance find+consume — max_uses:1 with concurrent access
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_allowance_max_uses_atomic() {
    let store = Arc::new(AllowanceStore::new());

    // Create allowance with max_uses: 1
    let allowance = make_test_allowance(
        AllowancePattern::ServerTools {
            server: "filesystem".to_string(),
        },
        Some(1),
        None,
    );
    store.add_allowance(allowance).unwrap();

    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };

    // Spawn 10 concurrent tasks, each trying to find_matching_and_consume
    let mut handles = Vec::new();
    for _ in 0..10 {
        let store = Arc::clone(&store);
        let action = action.clone();
        handles.push(tokio::spawn(async move {
            store.find_matching_and_consume(&action, None)
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let successes = results
        .into_iter()
        .filter(|r| r.as_ref().unwrap().is_some())
        .count();

    // Exactly 1 task should get Some, the rest should get None
    assert_eq!(
        successes, 1,
        "exactly 1 of 10 concurrent tasks should consume the single-use allowance"
    );
}

// ---------------------------------------------------------------------------
// 2. Atomic budget check+reserve — no overspend under concurrency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_budget_concurrent_no_overspend() {
    // Budget $100, each task tries to reserve $10
    let tracker = Arc::new(BudgetTracker::new(BudgetConfig::new(100.0, 10.0)));

    let mut handles = Vec::new();
    for _ in 0..20 {
        let tracker = Arc::clone(&tracker);
        handles.push(tokio::spawn(async move {
            tracker.check_and_reserve(10.0).is_allowed()
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let successes = results.into_iter().filter(|r| *r.as_ref().unwrap()).count();

    // At most 10 should succeed ($100 / $10 = 10)
    assert!(
        successes <= 10,
        "at most 10 of 20 tasks should succeed with $100 budget at $10 each, got {successes}"
    );
    // Total spent should not exceed $100
    assert!(
        tracker.spent() <= 100.0,
        "total spent should not exceed budget, got {}",
        tracker.spent()
    );
}

// ---------------------------------------------------------------------------
// 3. Workspace budget NOT bypassed by capability token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_budget_not_bypassed_by_capability() {
    let keypair = KeyPair::generate();
    let runtime_key = Arc::new(KeyPair::generate());
    let capability_store = Arc::new(CapabilityStore::in_memory());

    // Add a capability token for the file delete action
    let pattern = ResourcePattern::new("file:///workspace/temp.txt").unwrap();
    let token = CapabilityToken::create(
        pattern,
        vec![Permission::Delete],
        TokenScope::Session,
        keypair.key_id(),
        astrid_capabilities::AuditEntryId::new(),
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
    // Large session budget so it doesn't interfere
    let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::new(1000.0, 100.0)));
    let audit_log = Arc::new(AuditLog::in_memory(KeyPair::generate()));
    let session_id = SessionId::new();

    // Workspace budget of $5
    let ws_budget = Arc::new(WorkspaceBudgetTracker::new(Some(5.0), 80));

    approval_manager
        .register_handler(Arc::new(AutoApproveHandler) as Arc<dyn ApprovalHandler>)
        .await;

    let interceptor = SecurityInterceptor::new(
        capability_store,
        approval_manager,
        SecurityPolicy::default(),
        budget_tracker,
        audit_log,
        runtime_key,
        session_id,
        allowance_store,
        Some(std::path::PathBuf::from("/workspace")),
        Some(ws_budget),
    );

    // This action matches the capability token, but costs $10 > workspace budget $5
    let action = SensitiveAction::FileDelete {
        path: "/workspace/temp.txt".to_string(),
    };
    let result = interceptor
        .intercept(&action, "delete temp file", Some(10.0))
        .await;

    assert!(
        result.is_err(),
        "capability-authorized action should still be denied by workspace budget"
    );
    assert!(
        result.unwrap_err().to_string().contains("budget"),
        "error should mention budget"
    );
}

// ---------------------------------------------------------------------------
// 4. Allowance creation produces an audit entry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_allowance_creation_produces_audit_entry() {
    let capability_store = Arc::new(CapabilityStore::in_memory());
    let allowance_store = Arc::new(AllowanceStore::new());
    let deferred_queue = Arc::new(DeferredResolutionStore::new());
    let approval_manager = Arc::new(ApprovalManager::new(
        Arc::clone(&allowance_store),
        deferred_queue,
    ));
    let budget_tracker = Arc::new(BudgetTracker::default());
    let audit_log = Arc::new(AuditLog::in_memory(KeyPair::generate()));
    let runtime_key = Arc::new(KeyPair::generate());
    let session_id = SessionId::new();

    approval_manager
        .register_handler(Arc::new(SessionApproveHandler) as Arc<dyn ApprovalHandler>)
        .await;

    let interceptor = SecurityInterceptor::new(
        capability_store,
        approval_manager,
        SecurityPolicy::default(),
        budget_tracker,
        Arc::clone(&audit_log),
        runtime_key,
        session_id.clone(),
        allowance_store,
        None,
        None,
    );

    // FileRead triggers policy RequiresApproval -> handler returns ApproveSession
    let action = SensitiveAction::FileRead {
        path: "/workspace/data.txt".to_string(),
    };
    let result = interceptor.intercept(&action, "reading data", None).await;
    assert!(result.is_ok(), "intercept should succeed");

    // There should be audit entries for the allowance creation + the allowed action
    let count = audit_log.count_session(&session_id).unwrap();
    assert!(
        count >= 2,
        "should have at least 2 audit entries (allowance creation + allowed action), got {count}"
    );
}

// ---------------------------------------------------------------------------
// 5. Path traversal component check (edge cases)
// ---------------------------------------------------------------------------

#[test]
fn test_path_traversal_component_check() {
    let policy = SecurityPolicy::permissive();

    // Standard traversal — should be blocked
    let action = SensitiveAction::FileWriteOutsideSandbox {
        path: "/home/user/../../etc/passwd".to_string(),
    };
    assert!(
        policy.check(&action).is_blocked(),
        "standard path traversal should be blocked"
    );

    // Traversal at start — should be blocked
    let action = SensitiveAction::FileWriteOutsideSandbox {
        path: "../etc/passwd".to_string(),
    };
    assert!(
        policy.check(&action).is_blocked(),
        "traversal at start should be blocked"
    );

    // Traversal at end — should be blocked
    let action = SensitiveAction::FileWriteOutsideSandbox {
        path: "/home/user/..".to_string(),
    };
    assert!(
        policy.check(&action).is_blocked(),
        "traversal at end should be blocked"
    );

    // Triple dot — should NOT be blocked (not a traversal)
    let action = SensitiveAction::FileWriteOutsideSandbox {
        path: "/home/user/.../file.txt".to_string(),
    };
    assert!(
        !policy.check(&action).is_blocked(),
        "triple dot is not a traversal and should not be blocked"
    );

    // Normal path — should NOT be blocked
    let action = SensitiveAction::FileWriteOutsideSandbox {
        path: "/home/user/docs/file.txt".to_string(),
    };
    assert!(
        !policy.check(&action).is_blocked(),
        "normal path should not be blocked"
    );
}

// ---------------------------------------------------------------------------
// 6. Expired allowances cleaned on lookup
// ---------------------------------------------------------------------------

#[test]
fn test_expired_allowances_cleaned_on_lookup() {
    let store = AllowanceStore::new();

    // Insert an expired allowance
    let expired = make_test_allowance(
        AllowancePattern::ServerTools {
            server: "filesystem".to_string(),
        },
        None,
        Some(Timestamp::from_datetime(
            chrono::Utc::now() - chrono::Duration::hours(1),
        )),
    );
    store.add_allowance(expired).unwrap();
    assert_eq!(store.count(), 1, "store should have 1 entry before lookup");

    // Call find_matching_and_consume — should clean expired
    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    let result = store.find_matching_and_consume(&action, None);
    assert!(result.is_none(), "expired allowance should not match");

    // The expired allowance should have been cleaned from the store
    assert_eq!(
        store.count(),
        0,
        "expired allowance should be cleaned from store after find_matching_and_consume"
    );
}

// ---------------------------------------------------------------------------
// 7. Workspace budget check_and_reserve atomic — no overspend
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_budget_check_and_reserve_atomic() {
    // Workspace budget $50, each task tries to reserve $10
    let tracker = Arc::new(WorkspaceBudgetTracker::new(Some(50.0), 80));

    let mut handles = Vec::new();
    for _ in 0..10 {
        let tracker = Arc::clone(&tracker);
        handles.push(tokio::spawn(async move {
            tracker.check_and_reserve(10.0).is_allowed()
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let successes = results.into_iter().filter(|r| *r.as_ref().unwrap()).count();

    // At most 5 should succeed ($50 / $10 = 5)
    assert!(
        successes <= 5,
        "at most 5 of 10 tasks should succeed with $50 workspace budget at $10 each, got {successes}"
    );
    // Total spent should not exceed $50
    assert!(
        tracker.spent() <= 50.0,
        "total workspace spent should not exceed budget, got {}",
        tracker.spent()
    );
}
