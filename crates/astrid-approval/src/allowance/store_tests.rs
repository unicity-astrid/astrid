use super::*;
use crate::AllowancePattern;
use astrid_core::types::Timestamp;
use astrid_crypto::KeyPair;

/// Create a test allowance with the given pattern.
fn make_allowance(pattern: AllowancePattern, session_only: bool) -> Allowance {
    let keypair = KeyPair::generate();
    Allowance {
        id: AllowanceId::new(),
        action_pattern: pattern,
        created_at: Timestamp::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        session_only,
        workspace_root: None,
        signature: keypair.sign(b"test-allowance"),
    }
}

/// Create a limited-use test allowance.
fn make_limited_allowance(pattern: AllowancePattern, max_uses: u32) -> Allowance {
    let keypair = KeyPair::generate();
    Allowance {
        id: AllowanceId::new(),
        action_pattern: pattern,
        created_at: Timestamp::now(),
        expires_at: None,
        max_uses: Some(max_uses),
        uses_remaining: Some(max_uses),
        session_only: true,
        workspace_root: None,
        signature: keypair.sign(b"test-allowance"),
    }
}

#[test]
fn test_store_add_and_count() {
    let store = AllowanceStore::new();
    assert_eq!(store.count(), 0);

    let allowance = make_allowance(
        AllowancePattern::ServerTools {
            server: "fs".to_string(),
        },
        true,
    );
    store.add_allowance(allowance).unwrap();
    assert_eq!(store.count(), 1);
}

#[test]
fn test_store_find_matching() {
    let store = AllowanceStore::new();

    let allowance = make_allowance(
        AllowancePattern::ExactTool {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        },
        true,
    );
    let expected_id = allowance.id.clone();
    store.add_allowance(allowance).unwrap();

    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    let found = store.find_matching(&action, None);
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, expected_id);
}

#[test]
fn test_store_find_matching_no_match() {
    let store = AllowanceStore::new();

    let allowance = make_allowance(
        AllowancePattern::ExactTool {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        },
        true,
    );
    store.add_allowance(allowance).unwrap();

    let action = SensitiveAction::McpToolCall {
        server: "github".to_string(),
        tool: "create_issue".to_string(),
    };
    assert!(store.find_matching(&action, None).is_none());
}

#[test]
fn test_store_find_matching_skips_expired() {
    let store = AllowanceStore::new();

    let keypair = KeyPair::generate();
    let expired = Allowance {
        id: AllowanceId::new(),
        action_pattern: AllowancePattern::ServerTools {
            server: "filesystem".to_string(),
        },
        created_at: Timestamp::from_datetime(chrono::Utc::now() - chrono::Duration::hours(2)),
        expires_at: Some(Timestamp::from_datetime(
            chrono::Utc::now() - chrono::Duration::hours(1),
        )),
        max_uses: None,
        uses_remaining: None,
        session_only: true,
        workspace_root: None,
        signature: keypair.sign(b"test"),
    };
    store.add_allowance(expired).unwrap();

    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    assert!(store.find_matching(&action, None).is_none());
}

#[test]
fn test_store_find_matching_skips_exhausted() {
    let store = AllowanceStore::new();

    let mut allowance = make_limited_allowance(
        AllowancePattern::ServerTools {
            server: "filesystem".to_string(),
        },
        1,
    );
    let id = allowance.id.clone();
    // Pre-exhaust the uses
    allowance.uses_remaining = Some(0);
    store.add_allowance(allowance).unwrap();

    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    assert!(store.find_matching(&action, None).is_none());

    // Verify it's still in the store (not removed, just skipped)
    assert_eq!(store.count(), 1);
    // But consume_use on it still works (it's found by ID)
    assert!(store.consume_use(&id).is_ok());
}

#[test]
fn test_store_consume_use_limited() {
    let store = AllowanceStore::new();

    let allowance = make_limited_allowance(
        AllowancePattern::ServerTools {
            server: "fs".to_string(),
        },
        3,
    );
    let id = allowance.id.clone();
    store.add_allowance(allowance).unwrap();

    // 3 uses: consume down to 2, 1, 0
    assert_eq!(store.consume_use(&id).unwrap(), true); // 2 remaining
    assert_eq!(store.consume_use(&id).unwrap(), true); // 1 remaining
    assert_eq!(store.consume_use(&id).unwrap(), false); // 0 remaining (last use)

    // Saturates at 0
    assert_eq!(store.consume_use(&id).unwrap(), false);
}

#[test]
fn test_store_consume_use_unlimited() {
    let store = AllowanceStore::new();

    let allowance = make_allowance(
        AllowancePattern::ServerTools {
            server: "fs".to_string(),
        },
        true,
    );
    let id = allowance.id.clone();
    store.add_allowance(allowance).unwrap();

    // Unlimited: always returns true
    assert_eq!(store.consume_use(&id).unwrap(), true);
    assert_eq!(store.consume_use(&id).unwrap(), true);
}

#[test]
fn test_store_consume_use_not_found() {
    let store = AllowanceStore::new();
    let result = store.consume_use(&AllowanceId::new());
    assert!(result.is_err());
}

#[test]
fn test_store_cleanup_expired() {
    let store = AllowanceStore::new();

    let keypair = KeyPair::generate();

    // Add an expired allowance
    let expired = Allowance {
        id: AllowanceId::new(),
        action_pattern: AllowancePattern::ServerTools {
            server: "old".to_string(),
        },
        created_at: Timestamp::from_datetime(chrono::Utc::now() - chrono::Duration::hours(2)),
        expires_at: Some(Timestamp::from_datetime(
            chrono::Utc::now() - chrono::Duration::hours(1),
        )),
        max_uses: None,
        uses_remaining: None,
        session_only: true,
        workspace_root: None,
        signature: keypair.sign(b"expired"),
    };
    store.add_allowance(expired).unwrap();

    // Add a valid allowance
    let valid = make_allowance(
        AllowancePattern::ServerTools {
            server: "current".to_string(),
        },
        true,
    );
    store.add_allowance(valid).unwrap();

    assert_eq!(store.count(), 2);
    let removed = store.cleanup_expired();
    assert_eq!(removed, 1);
    assert_eq!(store.count(), 1);
}

#[test]
fn test_store_clear_session_allowances() {
    let store = AllowanceStore::new();

    // Session allowance
    let session = make_allowance(
        AllowancePattern::ServerTools {
            server: "session-server".to_string(),
        },
        true,
    );
    store.add_allowance(session).unwrap();

    // Non-session allowance
    let persistent = make_allowance(
        AllowancePattern::ServerTools {
            server: "persistent-server".to_string(),
        },
        false,
    );
    store.add_allowance(persistent).unwrap();

    assert_eq!(store.count(), 2);
    store.clear_session_allowances();
    assert_eq!(store.count(), 1);

    // The persistent one should still be matchable
    let action = SensitiveAction::McpToolCall {
        server: "persistent-server".to_string(),
        tool: "any_tool".to_string(),
    };
    assert!(store.find_matching(&action, None).is_some());

    // The session one should be gone
    let action = SensitiveAction::McpToolCall {
        server: "session-server".to_string(),
        tool: "any_tool".to_string(),
    };
    assert!(store.find_matching(&action, None).is_none());
}

#[test]
fn test_store_default() {
    let store = AllowanceStore::default();
    assert_eq!(store.count(), 0);
}

#[test]
fn test_store_debug() {
    let store = AllowanceStore::new();
    let debug = format!("{store:?}");
    assert!(debug.contains("AllowanceStore"));
    assert!(debug.contains("count"));
}
