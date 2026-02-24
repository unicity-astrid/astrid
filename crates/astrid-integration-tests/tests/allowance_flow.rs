//! Integration tests for allowance-based auto-approval flow.
//!
//! Tests that "Allow Session" creates an allowance that auto-approves
//! subsequent identical actions without re-prompting the user.

#![allow(clippy::arithmetic_side_effects)]

use std::sync::Arc;

use astrid_approval::deferred::DeferredResolutionStore;
use astrid_approval::manager::{ApprovalHandler, ApprovalManager};
use astrid_approval::request::{
    ApprovalDecision as InternalDecision, ApprovalRequest as InternalRequest,
    ApprovalResponse as InternalResponse,
};
use astrid_approval::{
    AllowanceStore, BudgetTracker, SecurityInterceptor, SecurityPolicy, SensitiveAction,
};
use astrid_audit::AuditLog;
use astrid_capabilities::CapabilityStore;
use astrid_core::SessionId;
use astrid_crypto::KeyPair;

/// Handler that returns "Approve Session" the first time, then denies.
struct SessionThenDenyHandler {
    call_count: std::sync::Mutex<usize>,
}

impl SessionThenDenyHandler {
    fn new() -> Self {
        Self {
            call_count: std::sync::Mutex::new(0),
        }
    }
}

#[async_trait::async_trait]
impl ApprovalHandler for SessionThenDenyHandler {
    async fn request_approval(&self, request: InternalRequest) -> Option<InternalResponse> {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        if *count == 1 {
            Some(InternalResponse::new(
                request.id,
                InternalDecision::ApproveSession,
            ))
        } else {
            Some(InternalResponse::new(
                request.id,
                InternalDecision::Deny {
                    reason: "second call should not reach handler".to_string(),
                },
            ))
        }
    }
    fn is_available(&self) -> bool {
        true
    }
}

/// "Allow Session" creates a session allowance that auto-approves the second
/// identical call without re-prompting the handler.
#[tokio::test]
async fn test_session_approval_flow() {
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

    let handler = Arc::new(SessionThenDenyHandler::new());
    approval_manager
        .register_handler(handler.clone() as Arc<dyn ApprovalHandler>)
        .await;

    let interceptor = SecurityInterceptor::new(
        capability_store,
        approval_manager,
        SecurityPolicy::default(),
        budget_tracker,
        audit_log,
        runtime_key,
        session_id,
        Arc::clone(&allowance_store),
        None,
        None,
    );

    let action = SensitiveAction::FileRead {
        path: "/workspace/data.txt".to_string(),
    };

    // First call: should be approved via "ApproveSession"
    let result1 = interceptor.intercept(&action, "reading data", None).await;
    assert!(result1.is_ok(), "first call should succeed");
    assert!(
        matches!(
            result1.unwrap().proof,
            astrid_approval::InterceptProof::SessionApproval { .. }
        ),
        "should be session approval"
    );

    // Verify allowance was created
    assert_eq!(
        allowance_store.count(),
        1,
        "session allowance should be stored"
    );

    // Second call: should be auto-approved by the stored allowance (handler not called)
    let result2 = interceptor.intercept(&action, "reading again", None).await;
    assert!(
        result2.is_ok(),
        "second call should be auto-approved by session allowance"
    );
    assert!(
        matches!(
            result2.unwrap().proof,
            astrid_approval::InterceptProof::Allowance { .. }
        ),
        "should be approved by existing allowance"
    );
}

/// Test that the `ApprovalManager`'s allowance-based approval works when
/// allowances are pre-populated in the store.
#[tokio::test]
async fn test_preexisting_allowance_auto_approves() {
    let allowance_store = Arc::new(AllowanceStore::new());
    let deferred_queue = Arc::new(DeferredResolutionStore::new());
    let approval_manager = Arc::new(ApprovalManager::new(
        Arc::clone(&allowance_store),
        deferred_queue,
    ));

    // Pre-populate an allowance for the MCP tool
    let keypair = KeyPair::generate();
    let allowance = astrid_approval::Allowance {
        id: astrid_approval::AllowanceId::new(),
        action_pattern: astrid_approval::AllowancePattern::ExactTool {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        },
        created_at: astrid_core::types::Timestamp::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        session_only: true,
        workspace_root: None,
        signature: keypair.sign(b"test-allowance"),
    };
    allowance_store.add_allowance(allowance).unwrap();

    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };

    // Should be auto-approved by the existing allowance (no handler needed)
    let outcome = approval_manager
        .check_approval(&action, "reading file", None)
        .await;
    assert!(
        outcome.is_allowed(),
        "should be allowed by existing allowance"
    );
}

/// Handler that returns "Approve Workspace" the first time, then denies.
struct WorkspaceThenDenyHandler {
    call_count: std::sync::Mutex<usize>,
}

impl WorkspaceThenDenyHandler {
    fn new() -> Self {
        Self {
            call_count: std::sync::Mutex::new(0),
        }
    }
}

#[async_trait::async_trait]
impl ApprovalHandler for WorkspaceThenDenyHandler {
    async fn request_approval(&self, request: InternalRequest) -> Option<InternalResponse> {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        if *count == 1 {
            Some(InternalResponse::new(
                request.id,
                InternalDecision::ApproveWorkspace,
            ))
        } else {
            Some(InternalResponse::new(
                request.id,
                InternalDecision::Deny {
                    reason: "second call should not reach handler".to_string(),
                },
            ))
        }
    }
    fn is_available(&self) -> bool {
        true
    }
}

/// "Allow Workspace" creates a workspace allowance (`session_only=false`) that
/// survives `clear_session_allowances()`.
#[tokio::test]
async fn test_workspace_approval_survives_session_clear() {
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

    let handler = Arc::new(WorkspaceThenDenyHandler::new());
    approval_manager
        .register_handler(handler.clone() as Arc<dyn ApprovalHandler>)
        .await;

    let interceptor = SecurityInterceptor::new(
        capability_store,
        approval_manager,
        SecurityPolicy::default(),
        budget_tracker,
        audit_log,
        runtime_key,
        session_id,
        Arc::clone(&allowance_store),
        Some(std::path::PathBuf::from("/workspace")),
        None,
    );

    // Use FileDelete which requires approval under the default policy
    let action = SensitiveAction::FileDelete {
        path: "/workspace/temp.txt".to_string(),
    };

    // First call: approved via "ApproveWorkspace"
    let result1 = interceptor
        .intercept(&action, "cleaning up temp files", None)
        .await;
    assert!(result1.is_ok(), "first call should succeed");

    // Verify allowance was created
    assert_eq!(
        allowance_store.count(),
        1,
        "workspace allowance should be stored"
    );

    // Clear session allowances — workspace allowance should survive
    allowance_store.clear_session_allowances();
    assert_eq!(
        allowance_store.count(),
        1,
        "workspace allowance (session_only=false) should survive clear_session_allowances"
    );

    // Second call: should still be auto-approved by the workspace allowance
    let result2 = interceptor
        .intercept(&action, "cleaning up again", None)
        .await;
    assert!(
        result2.is_ok(),
        "second call should be auto-approved by workspace allowance"
    );
}

/// A workspace allowance created in `/project-a` must NOT match actions
/// when the interceptor's `workspace_root` is `/project-b`.
#[tokio::test]
async fn test_workspace_allowance_does_not_match_different_workspace() {
    let allowance_store = Arc::new(AllowanceStore::new());

    // Create an allowance scoped to /project-a
    let keypair = KeyPair::generate();
    let allowance = astrid_approval::Allowance {
        id: astrid_approval::AllowanceId::new(),
        action_pattern: astrid_approval::AllowancePattern::FilePattern {
            pattern: "/project-a/src/**".to_string(),
            permission: astrid_core::types::Permission::Read,
        },
        created_at: astrid_core::types::Timestamp::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        session_only: false,
        workspace_root: Some(std::path::PathBuf::from("/project-a")),
        signature: keypair.sign(b"test-allowance"),
    };
    allowance_store.add_allowance(allowance).unwrap();

    let action = SensitiveAction::FileRead {
        path: "/project-a/src/main.rs".to_string(),
    };

    // Should match when workspace_root is /project-a
    let found = allowance_store.find_matching(&action, Some(std::path::Path::new("/project-a")));
    assert!(found.is_some(), "should match in same workspace");

    // Should NOT match when workspace_root is /project-b
    let found = allowance_store.find_matching(&action, Some(std::path::Path::new("/project-b")));
    assert!(
        found.is_none(),
        "workspace allowance from /project-a must not match in /project-b"
    );

    // Should NOT match when workspace_root is None
    let found = allowance_store.find_matching(&action, None);
    assert!(
        found.is_none(),
        "workspace-scoped allowance must not match when workspace_root is None"
    );
}
