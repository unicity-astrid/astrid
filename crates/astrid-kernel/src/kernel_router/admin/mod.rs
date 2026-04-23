//! Layer 6 admin dispatcher (issue #672).
//!
//! Subscribes to `astrid.v1.admin.*` and routes every variant of
//! [`AdminKernelRequest`] through the same capability-enforcement
//! preamble introduced in issue #670 (Layer 5). On allow, the mutating
//! handlers in [`handlers`] acquire
//! [`Kernel::admin_write_lock`](crate::Kernel::admin_write_lock) before
//! touching `profile.toml` / `groups.toml`, then atomically replace the
//! resolved config on the [`ArcSwap`](arc_swap::ArcSwap) backing
//! [`Kernel::groups`](crate::Kernel::groups) and/or invalidate the
//! matching [`PrincipalProfileCache`](astrid_capsule::profile_cache::PrincipalProfileCache)
//! entry.
//!
//! # Audit trail
//!
//! Every admin topic — allow or deny — appends an
//! [`AuditAction::AdminRequest`] entry. `method` is the wire name
//! (`"admin.agent.create"`, etc.); `target_principal` is `Some` for
//! variants that operate on another principal and `None` otherwise.

mod handlers;
#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use astrid_audit::{AuditOutcome, AuthorizationProof};
use astrid_core::principal::PrincipalId;
use astrid_events::ipc::IpcPayload;
use astrid_events::kernel_api::{AdminKernelRequest, AdminKernelResponse};
use tracing::warn;

use super::{
    AuthorityScope, authorize_request, publish_response, record_admin_audit, resolve_caller,
};

/// Admin IPC input topic prefix.
const ADMIN_TOPIC_PREFIX: &str = "astrid.v1.admin.";
/// Admin IPC response topic prefix (paired with [`ADMIN_TOPIC_PREFIX`]).
const ADMIN_RESPONSE_PREFIX: &str = "astrid.v1.admin.response.";

/// Spawn the admin dispatcher task. Mirrors [`super::spawn_kernel_router`]
/// but listens on `astrid.v1.admin.*` and parses
/// [`AdminKernelRequest`] payloads.
pub(super) fn spawn_admin_router(kernel: Arc<crate::Kernel>) -> tokio::task::JoinHandle<()> {
    let mut receiver = kernel.event_bus.subscribe_topic("astrid.v1.admin.*");

    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let astrid_events::AstridEvent::Ipc { message, .. } = &*event else {
                continue;
            };

            // Never loop back on our own response topic.
            if message.topic.starts_with(ADMIN_RESPONSE_PREFIX) {
                continue;
            }

            let IpcPayload::RawJson(val) = &message.payload else {
                continue;
            };

            match serde_json::from_value::<AdminKernelRequest>(val.clone()) {
                Ok(req) => {
                    let caller = resolve_caller(message);
                    handle_admin_request(&kernel, message.topic.clone(), caller, req).await;
                },
                Err(e) => {
                    warn!(
                        error = %e,
                        topic = %message.topic,
                        "Failed to parse AdminKernelRequest from IPC"
                    );
                },
            }
        }
    })
}

/// Compute the response topic for an incoming admin request topic.
fn admin_response_topic(input_topic: &str) -> String {
    input_topic.strip_prefix(ADMIN_TOPIC_PREFIX).map_or_else(
        || input_topic.to_string(),
        |suffix| format!("{ADMIN_RESPONSE_PREFIX}{suffix}"),
    )
}

/// Return the authority scope `req` exercises for `caller`.
///
/// Self-scoped when the target principal equals the caller
/// ([`AdminKernelRequest::QuotaGet`] / [`AdminKernelRequest::QuotaSet`]
/// / [`AdminKernelRequest::AgentList`] — the last scoped as "self" so
/// agents can see their own row). Everything else is cross-tenant,
/// including creation / group operations that are intrinsically global.
#[must_use]
pub fn resolve_admin_scope(req: &AdminKernelRequest, caller: &PrincipalId) -> AuthorityScope {
    match req {
        AdminKernelRequest::QuotaGet { principal }
        | AdminKernelRequest::QuotaSet { principal, .. } => {
            if principal == caller {
                AuthorityScope::Self_
            } else {
                AuthorityScope::Global
            }
        },
        AdminKernelRequest::AgentList => AuthorityScope::Self_,
        AdminKernelRequest::AgentCreate { .. }
        | AdminKernelRequest::AgentDelete { .. }
        | AdminKernelRequest::AgentEnable { .. }
        | AdminKernelRequest::AgentDisable { .. }
        | AdminKernelRequest::GroupCreate { .. }
        | AdminKernelRequest::GroupDelete { .. }
        | AdminKernelRequest::GroupModify { .. }
        | AdminKernelRequest::GroupList
        | AdminKernelRequest::CapsGrant { .. }
        | AdminKernelRequest::CapsRevoke { .. } => AuthorityScope::Global,
    }
}

/// Static capability string required to satisfy `req` under `scope`.
///
/// Pure function — the mapping can be unit-tested in isolation.
/// Every variant has an entry; there is no default-allow arm.
///
/// `self:*` forms apply when the target principal is the caller
/// themselves; admins operating on another principal need the
/// unscoped `quota:set` / `caps:grant` forms. Group admin is always
/// global — there is no "self" variant of `group:create`.
#[must_use]
pub fn required_capability_for_admin_request(
    req: &AdminKernelRequest,
    scope: AuthorityScope,
) -> &'static str {
    match (req, scope) {
        (AdminKernelRequest::AgentCreate { .. }, _) => "agent:create",
        (AdminKernelRequest::AgentDelete { .. }, _) => "agent:delete",
        (AdminKernelRequest::AgentEnable { .. }, _) => "agent:enable",
        (AdminKernelRequest::AgentDisable { .. }, _) => "agent:disable",
        (AdminKernelRequest::AgentList, AuthorityScope::Self_) => "self:agent:list",
        (AdminKernelRequest::AgentList, AuthorityScope::Global) => "agent:list",
        (AdminKernelRequest::QuotaSet { .. }, AuthorityScope::Self_) => "self:quota:set",
        (AdminKernelRequest::QuotaSet { .. }, AuthorityScope::Global) => "quota:set",
        (AdminKernelRequest::QuotaGet { .. }, AuthorityScope::Self_) => "self:quota:get",
        (AdminKernelRequest::QuotaGet { .. }, AuthorityScope::Global) => "quota:get",
        (AdminKernelRequest::GroupCreate { .. }, _) => "group:create",
        (AdminKernelRequest::GroupDelete { .. }, _) => "group:delete",
        (AdminKernelRequest::GroupModify { .. }, _) => "group:modify",
        (AdminKernelRequest::GroupList, _) => "group:list",
        (AdminKernelRequest::CapsGrant { .. }, _) => "caps:grant",
        (AdminKernelRequest::CapsRevoke { .. }, _) => "caps:revoke",
    }
}

/// Stable wire-name identifier for an [`AdminKernelRequest`] — used as
/// the `method` field on every [`AuditAction::AdminRequest`] entry.
#[must_use]
pub fn admin_request_method(req: &AdminKernelRequest) -> &'static str {
    match req {
        AdminKernelRequest::AgentCreate { .. } => "admin.agent.create",
        AdminKernelRequest::AgentDelete { .. } => "admin.agent.delete",
        AdminKernelRequest::AgentEnable { .. } => "admin.agent.enable",
        AdminKernelRequest::AgentDisable { .. } => "admin.agent.disable",
        AdminKernelRequest::AgentList => "admin.agent.list",
        AdminKernelRequest::QuotaSet { .. } => "admin.quota.set",
        AdminKernelRequest::QuotaGet { .. } => "admin.quota.get",
        AdminKernelRequest::GroupCreate { .. } => "admin.group.create",
        AdminKernelRequest::GroupDelete { .. } => "admin.group.delete",
        AdminKernelRequest::GroupModify { .. } => "admin.group.modify",
        AdminKernelRequest::GroupList => "admin.group.list",
        AdminKernelRequest::CapsGrant { .. } => "admin.caps.grant",
        AdminKernelRequest::CapsRevoke { .. } => "admin.caps.revoke",
    }
}

/// Borrow the target principal for audit purposes — `Some` only when the
/// request operates on a principal distinct from the caller.
#[must_use]
pub fn admin_target_principal(req: &AdminKernelRequest) -> Option<&PrincipalId> {
    match req {
        AdminKernelRequest::AgentDelete { principal }
        | AdminKernelRequest::AgentEnable { principal }
        | AdminKernelRequest::AgentDisable { principal }
        | AdminKernelRequest::QuotaSet { principal, .. }
        | AdminKernelRequest::QuotaGet { principal }
        | AdminKernelRequest::CapsGrant { principal, .. }
        | AdminKernelRequest::CapsRevoke { principal, .. } => Some(principal),
        AdminKernelRequest::AgentCreate { .. }
        | AdminKernelRequest::AgentList
        | AdminKernelRequest::GroupCreate { .. }
        | AdminKernelRequest::GroupDelete { .. }
        | AdminKernelRequest::GroupModify { .. }
        | AdminKernelRequest::GroupList => None,
    }
}

async fn handle_admin_request(
    kernel: &Arc<crate::Kernel>,
    topic: String,
    caller: PrincipalId,
    req: AdminKernelRequest,
) {
    let response_topic = admin_response_topic(&topic);
    let method = admin_request_method(&req);
    let scope = resolve_admin_scope(&req, &caller);
    let required_cap = required_capability_for_admin_request(&req, scope);
    let target = admin_target_principal(&req).cloned();

    match authorize_request(kernel, &caller, required_cap) {
        Ok(()) => {
            record_admin_audit(
                kernel,
                &caller,
                method,
                required_cap,
                target.clone(),
                AuthorizationProof::System {
                    reason: format!("policy allow: {caller} holds {required_cap}"),
                },
                AuditOutcome::success(),
            );
        },
        Err(e) => {
            warn!(
                security_event = true,
                method = method,
                principal = %caller,
                required = required_cap,
                "Permission check denied admin request"
            );
            record_admin_audit(
                kernel,
                &caller,
                method,
                required_cap,
                target,
                AuthorizationProof::Denied {
                    reason: e.to_string(),
                },
                AuditOutcome::failure(e.to_string()),
            );
            publish_response(
                kernel,
                response_topic,
                AdminKernelResponse::Error(format!(
                    "permission denied: missing capability {required_cap}"
                )),
            );
            return;
        },
    }

    let res = handlers::dispatch(kernel, req).await;
    publish_response(kernel, response_topic, res);
}
