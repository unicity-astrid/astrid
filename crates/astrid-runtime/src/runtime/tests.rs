use std::path::PathBuf;

use astrid_audit::AuthorizationProof;
use astrid_capabilities::AuditEntryId;
use astrid_core::RiskLevel;

use super::security::intercept_proof_to_auth_proof;
use super::workspace::{extract_paths_from_args, infer_operation, risk_level_for_operation};

#[test]
fn test_extract_paths_from_args() {
    let args = serde_json::json!({
        "path": "/home/user/file.txt",
        "content": "some data",
        "count": 42
    });
    let paths = extract_paths_from_args(&args);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("/home/user/file.txt"));
}

#[test]
fn test_extract_paths_ignores_non_path_values() {
    let args = serde_json::json!({
        "path": "not-a-path",
        "url": "https://example.com",
    });
    let paths = extract_paths_from_args(&args);
    assert!(paths.is_empty());
}

#[test]
fn test_extract_paths_file_uri() {
    let args = serde_json::json!({
        "uri": "file:///tmp/test.txt"
    });
    let paths = extract_paths_from_args(&args);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("/tmp/test.txt"));
}

#[test]
fn test_extract_paths_relative() {
    let args = serde_json::json!({
        "file": "./src/main.rs",
        "dir": "../other"
    });
    let paths = extract_paths_from_args(&args);
    assert_eq!(paths.len(), 2);
}

#[test]
fn test_infer_operation() {
    use astrid_workspace::escape::EscapeOperation;
    assert_eq!(infer_operation("read_file"), EscapeOperation::Read);
    assert_eq!(infer_operation("write_file"), EscapeOperation::Write);
    assert_eq!(infer_operation("create_directory"), EscapeOperation::Create);
    assert_eq!(infer_operation("delete_file"), EscapeOperation::Delete);
    assert_eq!(infer_operation("execute_command"), EscapeOperation::Execute);
    assert_eq!(infer_operation("list_files"), EscapeOperation::List);
    assert_eq!(infer_operation("unknown_tool"), EscapeOperation::Read);
}

#[test]
fn test_risk_level_for_operation() {
    use astrid_workspace::escape::EscapeOperation;
    assert_eq!(
        risk_level_for_operation(EscapeOperation::Read),
        RiskLevel::Medium
    );
    assert_eq!(
        risk_level_for_operation(EscapeOperation::Write),
        RiskLevel::High
    );
    assert_eq!(
        risk_level_for_operation(EscapeOperation::Delete),
        RiskLevel::Critical
    );
}

#[test]
fn test_intercept_proof_to_auth_proof_policy_allowed() {
    use astrid_approval::InterceptProof;
    let proof =
        intercept_proof_to_auth_proof(&InterceptProof::PolicyAllowed, [1; 8], "plugin:test:echo");
    match proof {
        AuthorizationProof::NotRequired { reason } => {
            assert!(reason.contains("policy auto-approved"));
            assert!(reason.contains("plugin:test:echo"));
        },
        other => panic!("expected NotRequired, got {other:?}"),
    }
}

#[test]
fn test_intercept_proof_to_auth_proof_user_approval() {
    use astrid_approval::InterceptProof;
    let audit_id = AuditEntryId::new();
    let proof = intercept_proof_to_auth_proof(
        &InterceptProof::UserApproval {
            approval_audit_id: audit_id.clone(),
        },
        [2; 8],
        "ctx",
    );
    match proof {
        AuthorizationProof::UserApproval {
            user_id,
            approval_entry_id,
        } => {
            assert_eq!(user_id, [2; 8]);
            assert_eq!(approval_entry_id, Some(audit_id));
        },
        other => panic!("expected UserApproval, got {other:?}"),
    }
}

#[test]
fn test_intercept_proof_to_auth_proof_session_approval() {
    use astrid_approval::InterceptProof;
    let proof = intercept_proof_to_auth_proof(
        &InterceptProof::SessionApproval {
            allowance_id: astrid_approval::AllowanceId::new(),
        },
        [3; 8],
        "ctx",
    );
    match proof {
        AuthorizationProof::NotRequired { reason } => {
            assert!(reason.contains("session-scoped allowance"));
        },
        other => panic!("expected NotRequired for session approval, got {other:?}"),
    }
}

#[test]
fn test_intercept_proof_to_auth_proof_workspace_approval() {
    use astrid_approval::InterceptProof;
    let proof = intercept_proof_to_auth_proof(
        &InterceptProof::WorkspaceApproval {
            allowance_id: astrid_approval::AllowanceId::new(),
        },
        [4; 8],
        "ctx",
    );
    match proof {
        AuthorizationProof::NotRequired { reason } => {
            assert!(reason.contains("workspace-scoped allowance"));
        },
        other => panic!("expected NotRequired for workspace approval, got {other:?}"),
    }
}

#[test]
fn test_intercept_proof_to_auth_proof_capability() {
    use astrid_approval::InterceptProof;
    let token_id = astrid_core::TokenId::new();
    let proof = intercept_proof_to_auth_proof(
        &InterceptProof::Capability {
            token_id: token_id.clone(),
        },
        [5; 8],
        "ctx",
    );
    match proof {
        AuthorizationProof::Capability {
            token_id: id,
            token_hash,
        } => {
            assert_eq!(id, token_id);
            // Hash should be derived from token_id string, not empty bytes.
            let expected = astrid_crypto::ContentHash::hash(token_id.to_string().as_bytes());
            assert_eq!(token_hash, expected);
        },
        other => panic!("expected Capability, got {other:?}"),
    }
}

#[test]
fn test_intercept_proof_to_auth_proof_allowance() {
    use astrid_approval::InterceptProof;
    let proof = intercept_proof_to_auth_proof(
        &InterceptProof::Allowance {
            allowance_id: astrid_approval::AllowanceId::new(),
        },
        [6; 8],
        "plugin:test:echo",
    );
    match proof {
        AuthorizationProof::NotRequired { reason } => {
            assert!(reason.contains("pre-existing allowance"));
            assert!(reason.contains("plugin:test:echo"));
        },
        other => panic!("expected NotRequired for allowance, got {other:?}"),
    }
}

#[test]
fn test_intercept_proof_to_auth_proof_capability_created() {
    use astrid_approval::InterceptProof;
    let audit_id = AuditEntryId::new();
    let token_id = astrid_core::TokenId::new();
    let proof = intercept_proof_to_auth_proof(
        &InterceptProof::CapabilityCreated {
            token_id: token_id.clone(),
            approval_audit_id: audit_id.clone(),
        },
        [7; 8],
        "ctx",
    );
    // CapabilityCreated shares the UserApproval arm with UserApproval,
    // using the approval_audit_id (the token_id is intentionally ignored
    // since the approval event is what matters for audit linking).
    match proof {
        AuthorizationProof::UserApproval {
            user_id,
            approval_entry_id,
        } => {
            assert_eq!(user_id, [7; 8]);
            assert_eq!(approval_entry_id, Some(audit_id));
        },
        other => panic!("expected UserApproval for CapabilityCreated, got {other:?}"),
    }
}
