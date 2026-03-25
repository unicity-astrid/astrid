//! Identity host functions for WASM capsules.
//!
//! Provides `identity_resolve`, `identity_link`, `identity_unlink`,
//! `identity_create_user`, and `identity_list_links` host functions.

use crate::engine::wasm::bindings::astrid::capsule::identity;
use crate::engine::wasm::bindings::astrid::capsule::types::{
    IdentityCreateUserRequest, IdentityLinkRequest, IdentityListLinksRequest, IdentityOkResponse,
    IdentityResolveRequest, IdentityResolveResponse, IdentityUnlinkRequest,
};
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use crate::security::IdentityOperation;

impl identity::Host for HostState {
    fn identity_resolve(
        &mut self,
        request: IdentityResolveRequest,
    ) -> Result<IdentityResolveResponse, String> {
        let identity_store = self
            .identity_store
            .clone()
            .ok_or_else(|| "identity store not available".to_string())?;

        let security = self
            .security
            .clone()
            .ok_or_else(|| "security gate not available".to_string())?;

        let capsule_id = self.capsule_id.to_string();
        let runtime_handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        let result = util::bounded_block_on(&runtime_handle, &host_semaphore, async {
            security
                .check_identity(&capsule_id, IdentityOperation::Resolve)
                .await
                .map_err(astrid_storage::StorageError::Internal)?;

            identity_store
                .resolve(&request.platform, &request.platform_user_id)
                .await
                .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
        });

        match result {
            Ok(Some(user)) => Ok(IdentityResolveResponse {
                found: true,
                user_id: Some(user.id.to_string()),
                display_name: user.display_name,
                error: None,
            }),
            Ok(None) => Ok(IdentityResolveResponse {
                found: false,
                user_id: None,
                display_name: None,
                error: None,
            }),
            Err(e) => Ok(IdentityResolveResponse {
                found: false,
                user_id: None,
                display_name: None,
                error: Some(e.to_string()),
            }),
        }
    }

    fn identity_link(
        &mut self,
        request: IdentityLinkRequest,
    ) -> Result<IdentityOkResponse, String> {
        let user_id = match uuid::Uuid::parse_str(&request.astrid_user_id) {
            Ok(id) => id,
            Err(e) => {
                return Ok(IdentityOkResponse {
                    ok: false,
                    error: Some(format!("invalid UUID: {e}")),
                });
            },
        };

        let identity_store = self
            .identity_store
            .clone()
            .ok_or_else(|| "identity store not available".to_string())?;

        let security = self
            .security
            .clone()
            .ok_or_else(|| "security gate not available".to_string())?;

        let capsule_id = self.capsule_id.to_string();
        let runtime_handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        let result = util::bounded_block_on(&runtime_handle, &host_semaphore, async {
            security
                .check_identity(&capsule_id, IdentityOperation::Link)
                .await
                .map_err(astrid_storage::StorageError::Internal)?;

            identity_store
                .link(
                    &request.platform,
                    &request.platform_user_id,
                    user_id,
                    &request.method,
                )
                .await
                .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
        });

        match result {
            Ok(_link) => Ok(IdentityOkResponse {
                ok: true,
                error: None,
            }),
            Err(e) => Ok(IdentityOkResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        }
    }

    fn identity_unlink(
        &mut self,
        request: IdentityUnlinkRequest,
    ) -> Result<IdentityOkResponse, String> {
        let identity_store = self
            .identity_store
            .clone()
            .ok_or_else(|| "identity store not available".to_string())?;

        let security = self
            .security
            .clone()
            .ok_or_else(|| "security gate not available".to_string())?;

        let capsule_id = self.capsule_id.to_string();
        let runtime_handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        let result = util::bounded_block_on(&runtime_handle, &host_semaphore, async {
            security
                .check_identity(&capsule_id, IdentityOperation::Unlink)
                .await
                .map_err(astrid_storage::StorageError::Internal)?;

            identity_store
                .unlink(&request.platform, &request.platform_user_id)
                .await
                .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
        });

        match result {
            Ok(_removed) => Ok(IdentityOkResponse {
                ok: true,
                error: None,
            }),
            Err(e) => Ok(IdentityOkResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        }
    }

    fn identity_create_user(
        &mut self,
        request: IdentityCreateUserRequest,
    ) -> Result<IdentityOkResponse, String> {
        let identity_store = self
            .identity_store
            .clone()
            .ok_or_else(|| "identity store not available".to_string())?;

        let security = self
            .security
            .clone()
            .ok_or_else(|| "security gate not available".to_string())?;

        let capsule_id = self.capsule_id.to_string();
        let runtime_handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        let result = util::bounded_block_on(&runtime_handle, &host_semaphore, async {
            security
                .check_identity(&capsule_id, IdentityOperation::CreateUser)
                .await
                .map_err(astrid_storage::StorageError::Internal)?;

            identity_store
                .create_user(request.display_name.as_deref())
                .await
                .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
        });

        match result {
            Ok(_user) => Ok(IdentityOkResponse {
                ok: true,
                error: None,
            }),
            Err(e) => Ok(IdentityOkResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        }
    }

    fn identity_list_links(
        &mut self,
        request: IdentityListLinksRequest,
    ) -> Result<IdentityOkResponse, String> {
        let user_id = match uuid::Uuid::parse_str(&request.astrid_user_id) {
            Ok(id) => id,
            Err(e) => {
                return Ok(IdentityOkResponse {
                    ok: false,
                    error: Some(format!("invalid UUID: {e}")),
                });
            },
        };

        let identity_store = self
            .identity_store
            .clone()
            .ok_or_else(|| "identity store not available".to_string())?;

        let security = self
            .security
            .clone()
            .ok_or_else(|| "security gate not available".to_string())?;

        let capsule_id = self.capsule_id.to_string();
        let runtime_handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        let result = util::bounded_block_on(&runtime_handle, &host_semaphore, async {
            security
                .check_identity(&capsule_id, IdentityOperation::ListLinks)
                .await
                .map_err(astrid_storage::StorageError::Internal)?;

            identity_store
                .list_links(user_id)
                .await
                .map_err(|e| astrid_storage::StorageError::Internal(e.to_string()))
        });

        match result {
            Ok(_links) => Ok(IdentityOkResponse {
                ok: true,
                error: None,
            }),
            Err(e) => Ok(IdentityOkResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        }
    }
}
