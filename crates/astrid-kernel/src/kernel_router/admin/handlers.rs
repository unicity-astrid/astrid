//! Layer 6 admin handler implementations (issue #672).
//!
//! Each handler assumes the caller has already passed the
//! [`super::handle_admin_request`] enforcement preamble; mutating
//! handlers acquire [`crate::Kernel::admin_write_lock`] before touching
//! disk state and invalidate the matching profile-cache entry after a
//! successful write.

use std::path::PathBuf;
use std::sync::Arc;

use astrid_core::capability_grammar::validate_capability;
use astrid_core::groups::{Group, GroupConfig};
use astrid_core::principal::PrincipalId;
use astrid_core::profile::{PrincipalProfile, ProfileError};
use astrid_events::kernel_api::{
    AdminKernelRequest, AdminKernelResponse, AgentSummary, GroupSummary,
};
use tracing::{info, warn};

/// Platform label used by the identity store for agent principals
/// created via [`AdminKernelRequest::AgentCreate`]. The per-principal
/// `platform_user_id` equals the `PrincipalId` string.
const AGENT_IDENTITY_PLATFORM: &str = "cli";

/// Dispatch an already-authorized [`AdminKernelRequest`] to the matching
/// handler.
pub(super) async fn dispatch(
    kernel: &Arc<crate::Kernel>,
    req: AdminKernelRequest,
) -> AdminKernelResponse {
    match req {
        AdminKernelRequest::AgentCreate {
            name,
            groups,
            grants,
        } => agent_create(kernel, name, groups, grants).await,
        AdminKernelRequest::AgentDelete { principal } => agent_delete(kernel, principal).await,
        AdminKernelRequest::AgentEnable { principal } => {
            agent_set_enabled(kernel, principal, true).await
        },
        AdminKernelRequest::AgentDisable { principal } => {
            agent_set_enabled(kernel, principal, false).await
        },
        AdminKernelRequest::AgentList => agent_list(kernel),
        AdminKernelRequest::QuotaSet { principal, quotas } => {
            quota_set(kernel, principal, quotas).await
        },
        AdminKernelRequest::QuotaGet { principal } => quota_get(kernel, &principal),
        AdminKernelRequest::GroupCreate {
            name,
            capabilities,
            description,
            unsafe_admin,
        } => group_create(kernel, name, capabilities, description, unsafe_admin).await,
        AdminKernelRequest::GroupDelete { name } => group_delete(kernel, name).await,
        AdminKernelRequest::GroupModify {
            name,
            capabilities,
            description,
            unsafe_admin,
        } => group_modify(kernel, name, capabilities, description, unsafe_admin).await,
        AdminKernelRequest::GroupList => group_list(kernel),
        AdminKernelRequest::CapsGrant {
            principal,
            capabilities,
        } => mutate_caps(kernel, &principal, capabilities, CapsMutation::Grant).await,
        AdminKernelRequest::CapsRevoke {
            principal,
            capabilities,
        } => mutate_caps(kernel, &principal, capabilities, CapsMutation::Revoke).await,
    }
}

// ── Agent lifecycle ────────────────────────────────────────────────────

async fn agent_create(
    kernel: &Arc<crate::Kernel>,
    name: String,
    groups: Vec<String>,
    grants: Vec<String>,
) -> AdminKernelResponse {
    let principal = match PrincipalId::new(name.clone()) {
        Ok(p) => p,
        Err(e) => return err_bad_input(format!("invalid principal name: {e}")),
    };

    // Reject bootstrap name: the `default` principal is seeded by
    // bootstrap_cli_root_user and must not be re-created through the
    // admin surface.
    if principal == PrincipalId::default() {
        return err_bad_input(format!(
            "principal {name:?} is reserved for single-tenant bootstrap"
        ));
    }

    let _guard = kernel.admin_write_lock.lock().await;
    let profile_path = principal_profile_path(kernel, &principal);

    // Collision: a profile on disk means this principal already exists.
    if profile_path.exists() {
        return err_bad_input(format!("principal {principal} already exists"));
    }

    let resolved_groups = if groups.is_empty() {
        vec![astrid_core::groups::BUILTIN_AGENT.to_string()]
    } else {
        groups
    };
    let profile = PrincipalProfile {
        groups: resolved_groups,
        grants,
        ..PrincipalProfile::default()
    };

    if let Err(e) = profile.validate() {
        return err_bad_input(format!("profile rejected: {e}"));
    }

    let user = match kernel
        .identity_store
        .create_user(Some(principal.as_str()))
        .await
    {
        Ok(u) => u,
        Err(e) => return err_internal(format!("identity store create_user failed: {e}")),
    };
    if let Err(e) = kernel
        .identity_store
        .link(
            AGENT_IDENTITY_PLATFORM,
            principal.as_str(),
            user.id,
            "system",
        )
        .await
    {
        // Best-effort rollback so partial state doesn't persist.
        let _ = kernel.identity_store.delete_user(user.id).await;
        return err_internal(format!("identity store link failed: {e}"));
    }

    if let Err(e) = profile.save_to_path(&profile_path) {
        let _ = kernel
            .identity_store
            .unlink(AGENT_IDENTITY_PLATFORM, principal.as_str())
            .await;
        let _ = kernel.identity_store.delete_user(user.id).await;
        return err_internal(format!("profile save failed: {e}"));
    }

    info!(%principal, user_id = %user.id, "Layer 6 agent.create");
    success_json(serde_json::json!({
        "principal": principal.as_str(),
        "astrid_user_id": user.id,
    }))
}

async fn agent_delete(kernel: &Arc<crate::Kernel>, principal: PrincipalId) -> AdminKernelResponse {
    if principal == PrincipalId::default() {
        return err_bad_input(
            "cannot delete the `default` principal — it is the single-tenant bootstrap anchor"
                .to_string(),
        );
    }

    let _guard = kernel.admin_write_lock.lock().await;

    // Resolve the link first so we know which user-record to delete.
    let resolved = match kernel
        .identity_store
        .resolve(AGENT_IDENTITY_PLATFORM, principal.as_str())
        .await
    {
        Ok(user) => user,
        Err(e) => return err_internal(format!("identity store resolve failed: {e}")),
    };
    // Unlink before delete_user so a concurrent `resolve` can't return
    // a dangling user id in the narrow window between the two calls.
    if let Err(e) = kernel
        .identity_store
        .unlink(AGENT_IDENTITY_PLATFORM, principal.as_str())
        .await
    {
        return err_internal(format!("identity store unlink failed: {e}"));
    }
    if let Some(user) = resolved
        && let Err(e) = kernel.identity_store.delete_user(user.id).await
    {
        return err_internal(format!("identity store delete_user failed: {e}"));
    }

    // Invalidate cache so subsequent authz checks for this principal
    // re-read from disk. The home directory itself is NOT scrubbed —
    // reclamation is an ops concern.
    kernel.profile_cache.invalidate(&principal);

    info!(%principal, "Layer 6 agent.delete");
    success_json(serde_json::json!({ "principal": principal.as_str() }))
}

async fn agent_set_enabled(
    kernel: &Arc<crate::Kernel>,
    principal: PrincipalId,
    enabled: bool,
) -> AdminKernelResponse {
    let _guard = kernel.admin_write_lock.lock().await;
    let path = principal_profile_path(kernel, &principal);
    let mut profile = match PrincipalProfile::load_from_path(&path) {
        Ok(p) => p,
        Err(e) => return err_profile(&principal, &e),
    };
    if profile.enabled == enabled {
        // No-op but still invalidate cache so the invariant "post-write
        // reads see current disk state" holds unconditionally.
        kernel.profile_cache.invalidate(&principal);
        return success_json(serde_json::json!({
            "principal": principal.as_str(),
            "enabled": enabled,
            "changed": false,
        }));
    }
    profile.enabled = enabled;
    if let Err(e) = profile.save_to_path(&path) {
        return err_profile(&principal, &e);
    }
    kernel.profile_cache.invalidate(&principal);
    success_json(serde_json::json!({
        "principal": principal.as_str(),
        "enabled": enabled,
        "changed": true,
    }))
}

fn agent_list(kernel: &Arc<crate::Kernel>) -> AdminKernelResponse {
    let home_dir = kernel.astrid_home.home_dir();
    let entries = match std::fs::read_dir(&home_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return AdminKernelResponse::AgentList(Vec::new());
        },
        Err(e) => {
            return err_internal(format!("failed to read {}: {e}", home_dir.display()));
        },
    };

    let mut summaries = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(principal) = PrincipalId::new(name) else {
            continue;
        };
        let profile = match kernel.profile_cache.resolve(&principal) {
            Ok(p) => p,
            Err(e) => {
                warn!(%principal, error = %e, "skipping agent.list entry with unreadable profile");
                continue;
            },
        };
        summaries.push(AgentSummary {
            principal,
            enabled: profile.enabled,
            groups: profile.groups.clone(),
            grants: profile.grants.clone(),
            revokes: profile.revokes.clone(),
        });
    }
    summaries.sort_by(|a, b| a.principal.as_str().cmp(b.principal.as_str()));
    AdminKernelResponse::AgentList(summaries)
}

// ── Quotas ─────────────────────────────────────────────────────────────

async fn quota_set(
    kernel: &Arc<crate::Kernel>,
    principal: PrincipalId,
    quotas: astrid_core::profile::Quotas,
) -> AdminKernelResponse {
    // Validate before taking the write lock — quick reject on bad input.
    if let Err(e) = quotas.validate() {
        return err_bad_input(format!("quotas rejected: {e}"));
    }

    let _guard = kernel.admin_write_lock.lock().await;
    let path = principal_profile_path(kernel, &principal);
    let mut profile = match PrincipalProfile::load_from_path(&path) {
        Ok(p) => p,
        Err(e) => return err_profile(&principal, &e),
    };
    profile.quotas = quotas;
    if let Err(e) = profile.save_to_path(&path) {
        return err_profile(&principal, &e);
    }
    kernel.profile_cache.invalidate(&principal);
    success_json(serde_json::json!({ "principal": principal.as_str() }))
}

fn quota_get(kernel: &Arc<crate::Kernel>, principal: &PrincipalId) -> AdminKernelResponse {
    match kernel.profile_cache.resolve(principal) {
        Ok(profile) => AdminKernelResponse::Quotas(profile.quotas.clone()),
        Err(e) => err_profile(principal, &e),
    }
}

// ── Groups ─────────────────────────────────────────────────────────────

async fn group_create(
    kernel: &Arc<crate::Kernel>,
    name: String,
    capabilities: Vec<String>,
    description: Option<String>,
    unsafe_admin: bool,
) -> AdminKernelResponse {
    let group = Group {
        capabilities,
        description,
        unsafe_admin,
    };
    let _guard = kernel.admin_write_lock.lock().await;
    let current = kernel.groups.load_full();
    let next = match current.insert_custom_group(name, group) {
        Ok(n) => n,
        Err(e) => return err_bad_input(format!("group.create rejected: {e}")),
    };
    commit_group_config(kernel, next)
}

async fn group_delete(kernel: &Arc<crate::Kernel>, name: String) -> AdminKernelResponse {
    let _guard = kernel.admin_write_lock.lock().await;
    let current = kernel.groups.load_full();
    let next = match current.remove_group(&name) {
        Ok(n) => n,
        Err(e) => return err_bad_input(format!("group.delete rejected: {e}")),
    };
    commit_group_config(kernel, next)
}

// `Option<Option<String>>` intentionally encodes three states: `None` =
// keep existing description, `Some(None)` = clear it, `Some(Some(v))` =
// replace with `v`. Collapsing to a single `Option` would conflate "no
// change" with "clear" at the wire format. Clippy's `option_option` lint
// is overly cautious for partial-update APIs.
#[allow(clippy::option_option)]
async fn group_modify(
    kernel: &Arc<crate::Kernel>,
    name: String,
    capabilities: Option<Vec<String>>,
    description: Option<Option<String>>,
    unsafe_admin: Option<bool>,
) -> AdminKernelResponse {
    let _guard = kernel.admin_write_lock.lock().await;
    let current = kernel.groups.load_full();
    let next = match current.modify_custom_group(&name, capabilities, description, unsafe_admin) {
        Ok(n) => n,
        Err(e) => return err_bad_input(format!("group.modify rejected: {e}")),
    };
    commit_group_config(kernel, next)
}

fn group_list(kernel: &Arc<crate::Kernel>) -> AdminKernelResponse {
    let cfg = kernel.groups.load_full();
    let mut summaries: Vec<GroupSummary> = cfg
        .iter()
        .map(|(name, group)| GroupSummary {
            name: name.clone(),
            capabilities: group.capabilities.clone(),
            description: group.description.clone(),
            unsafe_admin: group.unsafe_admin,
            builtin: GroupConfig::is_builtin_name(name),
        })
        .collect();
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    AdminKernelResponse::GroupList(summaries)
}

/// Commit a new [`GroupConfig`] to disk and the
/// [`ArcSwap`](arc_swap::ArcSwap). Caller must hold the admin write lock.
fn commit_group_config(kernel: &Arc<crate::Kernel>, next: GroupConfig) -> AdminKernelResponse {
    let path = GroupConfig::path_for(&kernel.astrid_home);
    if let Err(e) = next.save_to_path(&path) {
        return err_internal(format!("groups.toml save failed: {e}"));
    }
    kernel.groups.store(Arc::new(next));
    success_json(serde_json::json!({ "status": "ok" }))
}

// ── Per-principal grants / revokes ─────────────────────────────────────

enum CapsMutation {
    Grant,
    Revoke,
}

async fn mutate_caps(
    kernel: &Arc<crate::Kernel>,
    principal: &PrincipalId,
    capabilities: Vec<String>,
    which: CapsMutation,
) -> AdminKernelResponse {
    if capabilities.is_empty() {
        return err_bad_input("capabilities must not be empty".to_string());
    }
    for cap in &capabilities {
        if let Err(e) = validate_capability(cap) {
            return err_bad_input(format!("capability {cap:?} rejected: {e}"));
        }
    }

    let _guard = kernel.admin_write_lock.lock().await;
    let path = principal_profile_path(kernel, principal);
    let mut profile = match PrincipalProfile::load_from_path(&path) {
        Ok(p) => p,
        Err(e) => return err_profile(principal, &e),
    };

    // Grant-after-revoke must NOT clear the matching revoke — Layer 5
    // precedence is revoke > grant, so we just append. Revoke-after-grant
    // leaves the grant in place; the revoke wins at check time.
    match which {
        CapsMutation::Grant => profile.grants.extend(capabilities.iter().cloned()),
        CapsMutation::Revoke => profile.revokes.extend(capabilities.iter().cloned()),
    }

    if let Err(e) = profile.save_to_path(&path) {
        return err_profile(principal, &e);
    }
    kernel.profile_cache.invalidate(principal);
    success_json(serde_json::json!({
        "principal": principal.as_str(),
        "capabilities": capabilities,
    }))
}

// ── Helpers ────────────────────────────────────────────────────────────

fn principal_profile_path(kernel: &Arc<crate::Kernel>, principal: &PrincipalId) -> PathBuf {
    let ph = kernel.astrid_home.principal_home(principal);
    PrincipalProfile::path_for(&ph)
}

fn err_bad_input(msg: String) -> AdminKernelResponse {
    warn!(error = %msg, "admin request rejected: bad input");
    AdminKernelResponse::Error(msg)
}

fn err_internal(msg: String) -> AdminKernelResponse {
    warn!(error = %msg, "admin request failed: internal error");
    AdminKernelResponse::Error(msg)
}

fn err_profile(principal: &PrincipalId, e: &ProfileError) -> AdminKernelResponse {
    err_internal(format!("profile error for {principal}: {e}"))
}

fn success_json(val: serde_json::Value) -> AdminKernelResponse {
    AdminKernelResponse::Success(val)
}
