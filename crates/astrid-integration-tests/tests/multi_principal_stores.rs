//! Integration tests for Layer 4 cross-principal isolation (issue #668).
//!
//! These tests exercise the principal-scoped `AllowanceStore` and
//! `CapabilityStore` end-to-end: two distinct principals share the same
//! stores, approve or mint the same pattern, and must never see each
//! other's grants.

#![allow(clippy::arithmetic_side_effects)]

use std::sync::Arc;

use astrid_approval::{Allowance, AllowanceId, AllowancePattern, AllowanceStore, SensitiveAction};
use astrid_capabilities::{CapabilityStore, CapabilityToken, ResourcePattern, TokenScope};
use astrid_core::principal::PrincipalId;
use astrid_core::types::{Permission, Timestamp};
use astrid_crypto::KeyPair;

fn alice() -> PrincipalId {
    PrincipalId::new("alice").unwrap()
}

fn bob() -> PrincipalId {
    PrincipalId::new("bob").unwrap()
}

fn build_allowance(principal: PrincipalId, pattern: AllowancePattern) -> Allowance {
    let keypair = KeyPair::generate();
    Allowance {
        id: AllowanceId::new(),
        principal,
        action_pattern: pattern,
        created_at: Timestamp::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        session_only: true,
        workspace_root: None,
        signature: keypair.sign(b"multi-principal-test"),
    }
}

#[test]
fn alice_allowance_never_matches_bob_invocation() {
    let store = AllowanceStore::new();
    let pattern = AllowancePattern::ExactTool {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    store
        .add_allowance(build_allowance(alice(), pattern))
        .unwrap();

    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };

    assert!(store.find_matching(&alice(), &action, None).is_some());
    assert!(store.find_matching(&bob(), &action, None).is_none());
    assert!(
        store
            .find_matching_and_consume(&bob(), &action, None)
            .is_none()
    );
}

#[test]
fn alice_disconnect_does_not_clear_bob_session_allowances() {
    let store = AllowanceStore::new();
    let pattern_alice = AllowancePattern::ServerTools {
        server: "alice-srv".to_string(),
    };
    let pattern_bob = AllowancePattern::ServerTools {
        server: "bob-srv".to_string(),
    };
    store
        .add_allowance(build_allowance(alice(), pattern_alice))
        .unwrap();
    store
        .add_allowance(build_allowance(bob(), pattern_bob))
        .unwrap();

    assert_eq!(store.count(), 2);

    // Alice disconnects. Her session allowance must vanish; Bob's survives.
    store.clear_session_allowances(&alice());
    assert_eq!(store.count_for(&alice()), 0);
    assert_eq!(store.count_for(&bob()), 1);

    let bob_action = SensitiveAction::McpToolCall {
        server: "bob-srv".to_string(),
        tool: "anything".to_string(),
    };
    assert!(store.find_matching(&bob(), &bob_action, None).is_some());
}

#[test]
fn bob_minted_token_does_not_authorize_alice() {
    let runtime = KeyPair::generate();
    let store = CapabilityStore::in_memory();

    let token = CapabilityToken::create(
        ResourcePattern::exact("mcp://filesystem:read_file").unwrap(),
        vec![Permission::Invoke],
        TokenScope::Session,
        runtime.key_id(),
        astrid_capabilities::AuditEntryId::new(),
        &runtime,
        None,
        bob(),
    );
    store.add(token).unwrap();

    // Bob can consume his own token.
    assert!(store.has_capability(&bob(), "mcp://filesystem:read_file", Permission::Invoke));
    // Alice cannot — even though the resource pattern matches.
    assert!(!store.has_capability(&alice(), "mcp://filesystem:read_file", Permission::Invoke));
    assert!(
        store
            .find_capability(&alice(), "mcp://filesystem:read_file", Permission::Invoke)
            .is_none()
    );
}

#[test]
fn revocation_is_global_across_principals() {
    // Revocation is a property of the token's identity, not the caller.
    // Layer 4 keeps this invariant: revoking Bob's token revokes it for
    // every principal that might ever hold it.
    let runtime = KeyPair::generate();
    let store = CapabilityStore::in_memory();

    let token = CapabilityToken::create(
        ResourcePattern::exact("mcp://test:tool").unwrap(),
        vec![Permission::Invoke],
        TokenScope::Session,
        runtime.key_id(),
        astrid_capabilities::AuditEntryId::new(),
        &runtime,
        None,
        bob(),
    );
    let token_id = token.id.clone();
    store.add(token).unwrap();

    assert!(store.has_capability(&bob(), "mcp://test:tool", Permission::Invoke));
    store.revoke(&token_id).unwrap();
    // Revoked for Bob — and would stay revoked for Alice if she ever held
    // it. No cross-principal escape.
    assert!(!store.has_capability(&bob(), "mcp://test:tool", Permission::Invoke));
    assert!(matches!(
        store.get(&token_id),
        Err(astrid_capabilities::CapabilityError::TokenRevoked { .. })
    ));
}

#[tokio::test]
async fn overlay_registry_isolates_principal_writes() {
    use astrid_capabilities::DirHandle;
    use astrid_vfs::{OverlayVfsRegistry, Vfs};

    let workspace = tempfile::tempdir().unwrap();
    let registry = Arc::new(OverlayVfsRegistry::new(
        workspace.path().to_path_buf(),
        DirHandle::new(),
    ));

    let alice_vfs = registry.resolve(&alice()).await.unwrap();
    let bob_vfs = registry.resolve(&bob()).await.unwrap();
    let root = registry.root_handle().clone();

    let af = alice_vfs
        .open(&root, "shared.txt", true, true)
        .await
        .unwrap();
    alice_vfs.write(&af, b"ALICE").await.unwrap();
    alice_vfs.close(&af).await.unwrap();

    let bf = bob_vfs.open(&root, "shared.txt", true, true).await.unwrap();
    bob_vfs.write(&bf, b"BOB").await.unwrap();
    bob_vfs.close(&bf).await.unwrap();

    let ar = alice_vfs
        .open(&root, "shared.txt", false, false)
        .await
        .unwrap();
    let alice_bytes = alice_vfs.read(&ar).await.unwrap();
    alice_vfs.close(&ar).await.ok();

    let br = bob_vfs
        .open(&root, "shared.txt", false, false)
        .await
        .unwrap();
    let bob_bytes = bob_vfs.read(&br).await.unwrap();
    bob_vfs.close(&br).await.ok();

    assert_eq!(alice_bytes, b"ALICE");
    assert_eq!(bob_bytes, b"BOB");
}
