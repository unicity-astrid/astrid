//! Integration tests for session save/load roundtrip.

mod common;

use astralis_test::MockLlmTurn;
use common::RuntimeTestHarness;

#[tokio::test]
async fn test_save_and_load_roundtrip() {
    let mut harness = RuntimeTestHarness::new(vec![MockLlmTurn::text("Hello back!")]);

    harness.run_turn("Hello").await.unwrap();

    let session_id = harness.session.id.clone();
    let original_prompt = harness.session.system_prompt.clone();
    let original_msg_count = harness.session.messages.len();
    let original_created_at = harness.session.created_at;

    // Save
    harness.runtime.save_session(&harness.session).unwrap();

    // Load
    let loaded = harness
        .runtime
        .load_session(&session_id)
        .unwrap()
        .expect("session should exist");

    assert_eq!(loaded.system_prompt, original_prompt);
    assert_eq!(loaded.messages.len(), original_msg_count);
    assert!(loaded.token_count > 0);
    assert_eq!(loaded.created_at, original_created_at);
}

#[tokio::test]
async fn test_ephemeral_state_not_persisted() {
    let harness = RuntimeTestHarness::new(vec![]);

    // Save the session (no turns run, just the fresh session)
    harness.runtime.save_session(&harness.session).unwrap();

    // Load it back
    let loaded = harness
        .runtime
        .load_session(&harness.session.id)
        .unwrap()
        .expect("session should exist");

    // Loaded session should have fresh (empty) security state
    // The allowance store, capabilities, and escape handler are all session-scoped
    // and should NOT be persisted — they're reconstructed fresh on load.
    assert_eq!(loaded.messages.len(), 0);
    // escape_handler starts fresh (no approved paths)
    assert!(
        !loaded
            .escape_handler
            .is_allowed(&std::path::PathBuf::from("/tmp/outside"))
    );
}
