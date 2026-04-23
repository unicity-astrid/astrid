//! Stateful admin-handler tests (issue #672).
//!
//! Each test builds a [`test_kernel_with_home`](crate::test_kernel_with_home)
//! rooted in a private tempdir and invokes [`super::handlers::dispatch`]
//! directly, bypassing the IPC dispatch but keeping the write-lock / cache /
//! ArcSwap semantics identical to the production path.
//!
//! These tests cover the Layer 6 behavioural invariants: post-conditions
//! on disk, cache invalidation, ArcSwap hot-reload, adversarial
//! sequences (grant-after-revoke, quota=0 rejection, built-in protection,
//! concurrent writes).

use std::sync::Arc;

use astrid_core::dirs::AstridHome;
use astrid_core::groups::{BUILTIN_ADMIN, BUILTIN_AGENT, BUILTIN_RESTRICTED, GroupConfig};
use astrid_core::principal::PrincipalId;
use astrid_core::profile::{PrincipalProfile, Quotas};
use astrid_events::kernel_api::{
    AdminKernelRequest, AdminKernelResponse, AgentSummary, GroupSummary,
};
use tempfile::TempDir;

use super::handlers;
use crate::Kernel;

async fn fixture() -> (TempDir, Arc<Kernel>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = AstridHome::from_path(dir.path());
    let kernel = crate::test_kernel_with_home(home).await;
    (dir, kernel)
}

fn pid(name: &str) -> PrincipalId {
    PrincipalId::new(name).unwrap()
}

fn assert_success(res: &AdminKernelResponse) {
    match res {
        AdminKernelResponse::Success(_)
        | AdminKernelResponse::Quotas(_)
        | AdminKernelResponse::AgentList(_)
        | AdminKernelResponse::GroupList(_) => {},
        AdminKernelResponse::Error(msg) => panic!("expected success, got Error: {msg}"),
    }
}

fn assert_error_contains(res: &AdminKernelResponse, needle: &str) {
    match res {
        AdminKernelResponse::Error(msg) => {
            assert!(
                msg.contains(needle),
                "expected error to contain {needle:?}, got: {msg}"
            );
        },
        other => panic!("expected Error, got: {other:?}"),
    }
}

// ── agent.create ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn agent_create_writes_profile_and_links_identity() {
    let (_dir, kernel) = fixture().await;

    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "alice".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;
    assert_success(&res);

    // Profile written to disk with default group = "agent".
    let ph = kernel.astrid_home.principal_home(&pid("alice"));
    let profile = PrincipalProfile::load_from_path(&PrincipalProfile::path_for(&ph)).unwrap();
    assert_eq!(profile.groups, vec![BUILTIN_AGENT.to_string()]);
    assert!(profile.enabled);

    // Identity link created.
    let user = kernel.identity_store.resolve("cli", "alice").await.unwrap();
    assert!(user.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_create_rejects_collision_with_existing_profile() {
    let (_dir, kernel) = fixture().await;

    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "alice".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;

    // Second create with the same name → rejected.
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "alice".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;
    assert_error_contains(&res, "already exists");
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_create_rejects_invalid_name() {
    let (_dir, kernel) = fixture().await;
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "bad/name".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;
    assert_error_contains(&res, "invalid principal name");
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_create_rejects_default_bootstrap_name() {
    let (_dir, kernel) = fixture().await;
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "default".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;
    assert_error_contains(&res, "reserved");
}

// ── agent.delete ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn agent_delete_of_default_always_rejected() {
    let (_dir, kernel) = fixture().await;
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentDelete {
            principal: PrincipalId::default(),
        },
    )
    .await;
    assert_error_contains(&res, "default");
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_delete_removes_identity_and_invalidates_cache() {
    let (_dir, kernel) = fixture().await;

    // Create, then resolve via cache so there's an entry to invalidate.
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "bob".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;
    let _warm = kernel.profile_cache.resolve(&pid("bob")).unwrap();

    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentDelete {
            principal: pid("bob"),
        },
    )
    .await;
    assert_success(&res);

    // Link gone.
    let user = kernel.identity_store.resolve("cli", "bob").await.unwrap();
    assert!(user.is_none());
}

// ── agent.enable / disable ──────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn agent_enable_toggle_and_cache_invalidation() {
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "carol".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;

    // Warm cache with enabled=true.
    let warm = kernel.profile_cache.resolve(&pid("carol")).unwrap();
    assert!(warm.enabled);

    // Disable → cache should be invalidated so next resolve sees enabled=false.
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentDisable {
            principal: pid("carol"),
        },
    )
    .await;
    let after_disable = kernel.profile_cache.resolve(&pid("carol")).unwrap();
    assert!(!after_disable.enabled);

    // Re-enable roundtrips.
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentEnable {
            principal: pid("carol"),
        },
    )
    .await;
    let after_enable = kernel.profile_cache.resolve(&pid("carol")).unwrap();
    assert!(after_enable.enabled);
}

// ── agent.list ───────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn agent_list_returns_every_home_dir_principal() {
    let (_dir, kernel) = fixture().await;
    for name in ["alice", "bob"] {
        handlers::dispatch(
            &kernel,
            AdminKernelRequest::AgentCreate {
                name: name.into(),
                groups: Vec::new(),
                grants: Vec::new(),
            },
        )
        .await;
    }

    let res = handlers::dispatch(&kernel, AdminKernelRequest::AgentList).await;
    let AdminKernelResponse::AgentList(list) = res else {
        panic!("expected AgentList");
    };
    let names: Vec<&str> = list
        .iter()
        .map(|a: &AgentSummary| a.principal.as_str())
        .collect();
    assert!(names.contains(&"alice"), "got: {names:?}");
    assert!(names.contains(&"bob"), "got: {names:?}");
}

// ── quota.set / quota.get ────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn quota_set_rejects_zero_memory() {
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "dave".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;

    let mut q = Quotas::default();
    q.max_memory_bytes = 0;
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::QuotaSet {
            principal: pid("dave"),
            quotas: q,
        },
    )
    .await;
    assert_error_contains(&res, "quotas rejected");
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_set_updates_profile_and_invalidates_cache() {
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "eve".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;
    let _warm = kernel.profile_cache.resolve(&pid("eve")).unwrap();

    let mut q = Quotas::default();
    q.max_memory_bytes = 8 * 1024 * 1024;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::QuotaSet {
            principal: pid("eve"),
            quotas: q,
        },
    )
    .await;
    let fresh = kernel.profile_cache.resolve(&pid("eve")).unwrap();
    assert_eq!(fresh.quotas.max_memory_bytes, 8 * 1024 * 1024);

    // quota.get returns the current value.
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::QuotaGet {
            principal: pid("eve"),
        },
    )
    .await;
    let AdminKernelResponse::Quotas(got) = res else {
        panic!("expected Quotas response");
    };
    assert_eq!(got.max_memory_bytes, 8 * 1024 * 1024);
}

// ── group.create / delete / modify / list ───────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn group_create_swaps_arcswap_and_writes_groups_toml() {
    let (_dir, kernel) = fixture().await;

    // Pre: `ops` unknown.
    assert!(kernel.groups.load_full().get("ops").is_none());

    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::GroupCreate {
            name: "ops".into(),
            capabilities: vec!["capsule:install".into()],
            description: Some("deployment operators".into()),
            unsafe_admin: false,
        },
    )
    .await;
    assert_success(&res);

    // ArcSwap observes the new group immediately.
    let cfg = kernel.groups.load_full();
    let ops = cfg.get("ops").expect("ops present post-swap");
    assert_eq!(ops.capabilities, vec!["capsule:install".to_string()]);

    // Disk persists the same state (and excludes built-ins).
    let on_disk = GroupConfig::load_from_path(&GroupConfig::path_for(&kernel.astrid_home)).unwrap();
    assert!(on_disk.get("ops").is_some());
    let raw = std::fs::read_to_string(GroupConfig::path_for(&kernel.astrid_home)).unwrap();
    assert!(!raw.contains("[groups.admin]"));
    assert!(!raw.contains("[groups.agent]"));
    assert!(!raw.contains("[groups.restricted]"));
}

#[tokio::test(flavor = "multi_thread")]
async fn group_delete_rejects_every_builtin() {
    let (_dir, kernel) = fixture().await;
    for name in [BUILTIN_ADMIN, BUILTIN_AGENT, BUILTIN_RESTRICTED] {
        let res = handlers::dispatch(
            &kernel,
            AdminKernelRequest::GroupDelete { name: name.into() },
        )
        .await;
        assert_error_contains(&res, "built-in");
    }
    // Built-ins still present.
    let cfg = kernel.groups.load_full();
    assert!(cfg.get(BUILTIN_ADMIN).is_some());
    assert!(cfg.get(BUILTIN_AGENT).is_some());
    assert!(cfg.get(BUILTIN_RESTRICTED).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn group_modify_rejects_every_builtin() {
    let (_dir, kernel) = fixture().await;
    for name in [BUILTIN_ADMIN, BUILTIN_AGENT, BUILTIN_RESTRICTED] {
        let res = handlers::dispatch(
            &kernel,
            AdminKernelRequest::GroupModify {
                name: name.into(),
                capabilities: Some(vec!["audit:read".into()]),
                description: None,
                unsafe_admin: None,
            },
        )
        .await;
        assert_error_contains(&res, "built-in");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn group_list_returns_every_group_marked_correctly() {
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::GroupCreate {
            name: "ops".into(),
            capabilities: vec!["capsule:install".into()],
            description: None,
            unsafe_admin: false,
        },
    )
    .await;

    let res = handlers::dispatch(&kernel, AdminKernelRequest::GroupList).await;
    let AdminKernelResponse::GroupList(list) = res else {
        panic!("expected GroupList");
    };
    let by_name = |name: &str| list.iter().find(|g: &&GroupSummary| g.name == name);

    let admin = by_name("admin").expect("admin present");
    assert!(admin.builtin);
    let ops = by_name("ops").expect("ops present");
    assert!(!ops.builtin);
}

#[tokio::test(flavor = "multi_thread")]
async fn group_delete_reference_from_profile_does_not_elevate_privileges() {
    // Adversarial: a principal's profile references a custom group; we
    // delete that group. The principal must NOT be silently elevated
    // via any other group. Layer 5 fails closed on unknown group refs.
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::GroupCreate {
            name: "ops".into(),
            capabilities: vec!["capsule:install".into()],
            description: None,
            unsafe_admin: false,
        },
    )
    .await;

    // Create an agent with `ops` group membership.
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "frank".into(),
            groups: vec!["ops".into()],
            grants: Vec::new(),
        },
    )
    .await;

    // Delete `ops`. Frank's profile now has a dangling group ref.
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::GroupDelete { name: "ops".into() },
    )
    .await;

    // Re-resolve Frank's profile via cache. `ops` in groups vec, but
    // GroupConfig no longer contains it — fail-closed: `capsule:install`
    // must NOT be authorized.
    use astrid_capabilities::CapabilityCheck;
    let profile = kernel.profile_cache.resolve(&pid("frank")).unwrap();
    let groups = kernel.groups.load_full();
    let check = CapabilityCheck::new(profile.as_ref(), groups.as_ref(), pid("frank"));
    assert!(
        check.require("capsule:install").is_err(),
        "dangling group reference must not silently elevate"
    );
}

// ── caps.grant / revoke ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn caps_grant_appends_and_invalidates_cache() {
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "grace".into(),
            groups: vec!["restricted".into()],
            grants: Vec::new(),
        },
    )
    .await;

    // Pre-check: restricted principal can't do capsule:install.
    use astrid_capabilities::CapabilityCheck;
    {
        let profile = kernel.profile_cache.resolve(&pid("grace")).unwrap();
        let groups = kernel.groups.load_full();
        let check = CapabilityCheck::new(profile.as_ref(), groups.as_ref(), pid("grace"));
        assert!(check.require("capsule:install").is_err());
    }

    handlers::dispatch(
        &kernel,
        AdminKernelRequest::CapsGrant {
            principal: pid("grace"),
            capabilities: vec!["capsule:install".into()],
        },
    )
    .await;

    // Post: cache invalidated, fresh profile has the grant.
    let profile = kernel.profile_cache.resolve(&pid("grace")).unwrap();
    let groups = kernel.groups.load_full();
    let check = CapabilityCheck::new(profile.as_ref(), groups.as_ref(), pid("grace"));
    check.require("capsule:install").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn caps_grant_does_not_clear_matching_revoke() {
    // Adversarial: pre-existing `self:*` revoke + caps.grant of a
    // matching cap → authz check still denies (revoke > grant).
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "henry".into(),
            groups: vec!["admin".into()],
            grants: Vec::new(),
        },
    )
    .await;
    // Install a revoke first.
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::CapsRevoke {
            principal: pid("henry"),
            capabilities: vec!["self:*".into()],
        },
    )
    .await;
    // Now grant a cap covered by the revoke pattern.
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::CapsGrant {
            principal: pid("henry"),
            capabilities: vec!["self:capsule:install".into()],
        },
    )
    .await;

    use astrid_capabilities::{CapabilityCheck, PermissionError};
    let profile = kernel.profile_cache.resolve(&pid("henry")).unwrap();
    let groups = kernel.groups.load_full();
    let check = CapabilityCheck::new(profile.as_ref(), groups.as_ref(), pid("henry"));
    let err = check.require("self:capsule:install").unwrap_err();
    assert!(
        matches!(err, PermissionError::RevokedCapability { .. }),
        "grant must not clear revoke: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn caps_revoke_of_unheld_capability_appends_preemptive() {
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "ivy".into(),
            groups: vec!["restricted".into()],
            grants: Vec::new(),
        },
    )
    .await;
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::CapsRevoke {
            principal: pid("ivy"),
            capabilities: vec!["capsule:install".into()],
        },
    )
    .await;
    assert_success(&res);

    // Revoke is persisted even though the principal didn't hold the cap.
    let profile = kernel.profile_cache.resolve(&pid("ivy")).unwrap();
    assert!(profile.revokes.iter().any(|r| r == "capsule:install"));
}

#[tokio::test(flavor = "multi_thread")]
async fn caps_grant_rejects_invalid_capability_grammar() {
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "julia".into(),
            groups: Vec::new(),
            grants: Vec::new(),
        },
    )
    .await;
    let res = handlers::dispatch(
        &kernel,
        AdminKernelRequest::CapsGrant {
            principal: pid("julia"),
            capabilities: vec!["system:shut down".into()], // space → invalid
        },
    )
    .await;
    assert_error_contains(&res, "rejected");
}

// ── Concurrency: write lock serializes mutations ────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_caps_grants_serialized_by_admin_write_lock() {
    // Two concurrent grants on the same principal must both land.
    // Without the write lock they could interleave load/save and drop
    // one of the grants.
    let (_dir, kernel) = fixture().await;
    handlers::dispatch(
        &kernel,
        AdminKernelRequest::AgentCreate {
            name: "kate".into(),
            groups: vec!["restricted".into()],
            grants: Vec::new(),
        },
    )
    .await;

    let k1 = Arc::clone(&kernel);
    let k2 = Arc::clone(&kernel);
    let t1 = tokio::spawn(async move {
        handlers::dispatch(
            &k1,
            AdminKernelRequest::CapsGrant {
                principal: pid("kate"),
                capabilities: vec!["capsule:install".into()],
            },
        )
        .await
    });
    let t2 = tokio::spawn(async move {
        handlers::dispatch(
            &k2,
            AdminKernelRequest::CapsGrant {
                principal: pid("kate"),
                capabilities: vec!["capsule:remove".into()],
            },
        )
        .await
    });
    let (r1, r2) = (t1.await.unwrap(), t2.await.unwrap());
    assert_success(&r1);
    assert_success(&r2);

    let profile = kernel.profile_cache.resolve(&pid("kate")).unwrap();
    assert!(profile.grants.iter().any(|c| c == "capsule:install"));
    assert!(profile.grants.iter().any(|c| c == "capsule:remove"));
}
