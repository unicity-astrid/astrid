//! Identity host functions for WASM capsules.
//!
//! Provides `astrid_identity_resolve`, `astrid_identity_link`,
//! `astrid_identity_unlink`, `astrid_identity_create_user`, and
//! `astrid_identity_list_links` host functions.

use extism::{CurrentPlugin, Error, UserData, Val};
use serde::{Deserialize, Serialize};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use crate::security::IdentityOperation;

// ---------------------------------------------------------------------------
// JSON wire types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ResolveRequest {
    platform: String,
    platform_user_id: String,
}

#[derive(Serialize)]
struct ResolveResponse {
    found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
struct LinkRequest {
    platform: String,
    platform_user_id: String,
    astrid_user_id: String,
    method: String,
}

#[derive(Deserialize)]
struct UnlinkRequest {
    platform: String,
    platform_user_id: String,
}

#[derive(Deserialize)]
struct CreateUserRequest {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct ListLinksRequest {
    astrid_user_id: String,
}

#[derive(Serialize)]
struct LinkInfo {
    platform: String,
    platform_user_id: String,
    astrid_user_id: String,
    linked_at: String,
    method: String,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    removed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<LinkInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    links: Option<Vec<LinkInfo>>,
}

impl OkResponse {
    fn success() -> Self {
        Self {
            ok: true,
            error: None,
            user_id: None,
            removed: None,
            link: None,
            links: None,
        }
    }

    fn fail(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(msg.into()),
            user_id: None,
            removed: None,
            link: None,
            links: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: extract state fields needed for identity ops
// ---------------------------------------------------------------------------

struct IdentityContext {
    capsule_id: String,
    identity_store: std::sync::Arc<dyn astrid_storage::IdentityStore>,
    security: std::sync::Arc<dyn crate::security::CapsuleSecurityGate>,
    runtime_handle: tokio::runtime::Handle,
    host_semaphore: std::sync::Arc<tokio::sync::Semaphore>,
}

fn extract_identity_context(user_data: &UserData<HostState>) -> Result<IdentityContext, Error> {
    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let identity_store = state
        .identity_store
        .clone()
        .ok_or_else(|| Error::msg("identity store not available"))?;

    let security = state
        .security
        .clone()
        .ok_or_else(|| Error::msg("security gate not available"))?;

    Ok(IdentityContext {
        capsule_id: state.capsule_id.to_string(),
        identity_store,
        security,
        runtime_handle: state.runtime_handle.clone(),
        host_semaphore: state.host_semaphore.clone(),
    })
}

/// Write a JSON response to plugin memory and set the output.
fn write_json_response(
    plugin: &mut CurrentPlugin,
    outputs: &mut [Val],
    value: &impl Serialize,
) -> Result<(), Error> {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| Error::msg(format!("JSON serialization failed: {e}")))?;
    let mem = plugin.memory_new(&bytes)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// Host function implementations
// ---------------------------------------------------------------------------

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_identity_resolve_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let req: ResolveRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("invalid resolve request: {e}")))?;

    let ctx = extract_identity_context(&user_data)?;

    let result = util::bounded_block_on(&ctx.runtime_handle, &ctx.host_semaphore, async {
        ctx.security
            .check_identity(&ctx.capsule_id, IdentityOperation::Resolve)
            .await
            .map_err(astrid_storage::StorageError::Internal)?;

        ctx.identity_store
            .resolve(&req.platform, &req.platform_user_id)
            .await
            .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
    });

    let response = match result {
        Ok(Some(user)) => ResolveResponse {
            found: true,
            user_id: Some(user.id.to_string()),
            display_name: user.display_name,
            error: None,
        },
        Ok(None) => ResolveResponse {
            found: false,
            user_id: None,
            display_name: None,
            error: None,
        },
        Err(e) => ResolveResponse {
            found: false,
            user_id: None,
            display_name: None,
            error: Some(e.to_string()),
        },
    };

    write_json_response(plugin, outputs, &response)
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_identity_link_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let req: LinkRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("invalid link request: {e}")))?;

    let user_id = uuid::Uuid::parse_str(&req.astrid_user_id)
        .map_err(|e| Error::msg(format!("invalid UUID: {e}")))?;

    let ctx = extract_identity_context(&user_data)?;

    let result = util::bounded_block_on(&ctx.runtime_handle, &ctx.host_semaphore, async {
        ctx.security
            .check_identity(&ctx.capsule_id, IdentityOperation::Link)
            .await
            .map_err(astrid_storage::StorageError::Internal)?;

        ctx.identity_store
            .link(&req.platform, &req.platform_user_id, user_id, &req.method)
            .await
            .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
    });

    let response = match result {
        Ok(link) => {
            let mut resp = OkResponse::success();
            resp.link = Some(LinkInfo {
                platform: link.platform,
                platform_user_id: link.platform_user_id,
                astrid_user_id: link.astrid_user_id.to_string(),
                linked_at: link.linked_at.to_rfc3339(),
                method: link.method,
            });
            resp
        },
        Err(e) => OkResponse::fail(e.to_string()),
    };

    write_json_response(plugin, outputs, &response)
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_identity_unlink_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let req: UnlinkRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("invalid unlink request: {e}")))?;

    let ctx = extract_identity_context(&user_data)?;

    let result = util::bounded_block_on(&ctx.runtime_handle, &ctx.host_semaphore, async {
        ctx.security
            .check_identity(&ctx.capsule_id, IdentityOperation::Unlink)
            .await
            .map_err(astrid_storage::StorageError::Internal)?;

        ctx.identity_store
            .unlink(&req.platform, &req.platform_user_id)
            .await
            .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
    });

    let response = match result {
        Ok(removed) => {
            let mut resp = OkResponse::success();
            resp.removed = Some(removed);
            resp
        },
        Err(e) => OkResponse::fail(e.to_string()),
    };

    write_json_response(plugin, outputs, &response)
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_identity_create_user_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let req: CreateUserRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("invalid create_user request: {e}")))?;

    let ctx = extract_identity_context(&user_data)?;

    let result = util::bounded_block_on(&ctx.runtime_handle, &ctx.host_semaphore, async {
        ctx.security
            .check_identity(&ctx.capsule_id, IdentityOperation::CreateUser)
            .await
            .map_err(astrid_storage::StorageError::Internal)?;

        ctx.identity_store
            .create_user(req.display_name.as_deref())
            .await
            .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
    });

    let response = match result {
        Ok(user) => {
            let mut resp = OkResponse::success();
            resp.user_id = Some(user.id.to_string());
            resp
        },
        Err(e) => OkResponse::fail(e.to_string()),
    };

    write_json_response(plugin, outputs, &response)
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_identity_list_links_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let req: ListLinksRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("invalid list_links request: {e}")))?;

    let user_id = uuid::Uuid::parse_str(&req.astrid_user_id)
        .map_err(|e| Error::msg(format!("invalid UUID: {e}")))?;

    let ctx = extract_identity_context(&user_data)?;

    let result = util::bounded_block_on(&ctx.runtime_handle, &ctx.host_semaphore, async {
        ctx.security
            .check_identity(&ctx.capsule_id, IdentityOperation::ListLinks)
            .await
            .map_err(astrid_storage::StorageError::Internal)?;

        ctx.identity_store
            .list_links(user_id)
            .await
            .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
    });

    let response = match result {
        Ok(links) => {
            let mut resp = OkResponse::success();
            resp.links = Some(
                links
                    .into_iter()
                    .map(|l| LinkInfo {
                        platform: l.platform,
                        platform_user_id: l.platform_user_id,
                        astrid_user_id: l.astrid_user_id.to_string(),
                        linked_at: l.linked_at.to_rfc3339(),
                        method: l.method,
                    })
                    .collect(),
            );
            resp
        },
        Err(e) => OkResponse::fail(e.to_string()),
    };

    write_json_response(plugin, outputs, &response)
}
