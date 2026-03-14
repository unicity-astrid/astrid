//! Identity store for managing users and platform links.
//!
//! Provides an [`IdentityStore`] trait with a KV-backed implementation
//! ([`KvIdentityStore`]) that stores user records and platform links
//! in a [`ScopedKvStore`] with namespace `system:identity`.
//!
//! ## KV Key Scheme
//!
//! Keys use `/` as the separator. Both `platform` and `platform_user_id`
//! are validated to reject `/` and `\0` before key construction:
//!
//! - `user/{uuid}` - JSON-serialized [`AstridUserId`]
//! - `link/{platform}/{platform_user_id}` - JSON-serialized [`FrontendLink`]
//! - `name/{display_name}` - UUID string (name-to-UUID index for config resolution)

use std::fmt;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use astrid_core::identity::types::{AstridUserId, FrontendLink, normalize_platform};

use crate::kv::ScopedKvStore;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from identity store operations.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The specified user was not found.
    #[error("user not found: {0}")]
    UserNotFound(Uuid),

    /// The underlying storage operation failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// Input validation failed.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Identity store for managing users and platform links.
///
/// All operations are async because the backing store (KV) is async.
#[async_trait]
pub trait IdentityStore: Send + Sync + fmt::Debug {
    /// Create a new [`AstridUserId`]. Returns the created user.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::Storage`] if persistence fails.
    async fn create_user(&self, display_name: Option<&str>) -> Result<AstridUserId, IdentityError>;

    /// Look up a user by UUID. Returns `None` if not found.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::Storage`] if the read fails.
    async fn get_user(&self, id: Uuid) -> Result<Option<AstridUserId>, IdentityError>;

    /// Resolve a platform identity to an [`AstridUserId`].
    /// Returns `None` if no link exists for this platform + `user_id` pair.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::Storage`] if the read fails.
    /// Returns [`IdentityError::InvalidInput`] if platform or `user_id` is empty.
    async fn resolve(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<AstridUserId>, IdentityError>;

    /// Link a platform identity to an existing [`AstridUserId`].
    ///
    /// Uses upsert semantics: if a link already exists for this
    /// platform + `user_id`, it is updated to point to the new user.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::UserNotFound`] if the target user doesn't exist.
    /// Returns [`IdentityError::InvalidInput`] if any input is empty.
    /// Returns [`IdentityError::Storage`] if persistence fails.
    async fn link(
        &self,
        platform: &str,
        platform_user_id: &str,
        astrid_user_id: Uuid,
        method: &str,
    ) -> Result<FrontendLink, IdentityError>;

    /// Remove a platform link. Returns `true` if the link existed.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::InvalidInput`] if platform or `user_id` is empty.
    /// Returns [`IdentityError::Storage`] if the delete fails.
    async fn unlink(&self, platform: &str, platform_user_id: &str) -> Result<bool, IdentityError>;

    /// List all links for a given [`AstridUserId`].
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::Storage`] if the scan fails.
    async fn list_links(&self, astrid_user_id: Uuid) -> Result<Vec<FrontendLink>, IdentityError>;

    /// Look up a user by display name. Returns `None` if not found.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::Storage`] if the read fails.
    async fn get_user_by_name(&self, name: &str) -> Result<Option<AstridUserId>, IdentityError>;
}

// ---------------------------------------------------------------------------
// KV-backed implementation
// ---------------------------------------------------------------------------

/// KV-backed identity store.
///
/// Uses a [`ScopedKvStore`] (typically namespace `system:identity`) to persist
/// user records and platform links. All data is JSON-serialized.
#[derive(Clone)]
pub struct KvIdentityStore {
    kv: ScopedKvStore,
}

impl fmt::Debug for KvIdentityStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KvIdentityStore")
            .field("namespace", &self.kv.namespace())
            .finish()
    }
}

impl KvIdentityStore {
    /// Create a new KV-backed identity store.
    #[must_use]
    pub fn new(kv: ScopedKvStore) -> Self {
        Self { kv }
    }

    /// Build the KV key for a user record.
    fn user_key(id: Uuid) -> String {
        format!("user/{id}")
    }

    /// Build the KV key for a platform link.
    fn link_key(platform: &str, platform_user_id: &str) -> String {
        format!("link/{platform}/{platform_user_id}")
    }

    /// Build the KV key for a name-to-UUID index entry.
    fn name_key(name: &str) -> String {
        format!("name/{name}")
    }

    /// Validate that a string is non-empty.
    fn validate_non_empty(value: &str, field: &str) -> Result<(), IdentityError> {
        if value.trim().is_empty() {
            return Err(IdentityError::InvalidInput(format!(
                "{field} must not be empty"
            )));
        }
        Ok(())
    }

    /// Validate that a platform name is safe for use as a KV key component.
    ///
    /// Rejects empty strings and strings containing `/` or `\0`, which would
    /// allow key-path injection in the `link/{platform}/{platform_user_id}` scheme.
    fn validate_platform(value: &str) -> Result<(), IdentityError> {
        Self::validate_non_empty(value, "platform")?;
        if value.contains('/') || value.contains('\0') {
            return Err(IdentityError::InvalidInput(
                "platform must not contain '/' or null bytes".into(),
            ));
        }
        Ok(())
    }

    /// Validate that a platform user ID is safe for use as a KV key component.
    ///
    /// Rejects empty strings and strings containing `/` or `\0`, which would
    /// allow key-path injection in the `link/{platform}/{platform_user_id}` scheme.
    fn validate_platform_user_id(value: &str) -> Result<(), IdentityError> {
        Self::validate_non_empty(value, "platform_user_id")?;
        if value.contains('/') || value.contains('\0') {
            return Err(IdentityError::InvalidInput(
                "platform_user_id must not contain '/' or null bytes".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl IdentityStore for KvIdentityStore {
    async fn create_user(&self, display_name: Option<&str>) -> Result<AstridUserId, IdentityError> {
        let mut user = AstridUserId::new();
        if let Some(name) = display_name {
            user = user.with_display_name(name);
        }

        self.kv
            .set_json(&Self::user_key(user.id), &user)
            .await
            .map_err(|e| IdentityError::Storage(e.to_string()))?;

        // Index by display name if provided (skip if contains key-unsafe chars).
        // Note: this overwrites any existing name index entry. The name index is
        // a best-effort lookup for config resolution, not a uniqueness constraint.
        // Last writer wins - the most recently created user with a given name
        // will be found by `get_user_by_name`.
        if let Some(name) = display_name
            && !name.trim().is_empty()
            && !name.contains('/')
            && !name.contains('\0')
        {
            self.kv
                .set(
                    &Self::name_key(name.trim()),
                    user.id.to_string().into_bytes(),
                )
                .await
                .map_err(|e| IdentityError::Storage(e.to_string()))?;
        }

        Ok(user)
    }

    async fn get_user(&self, id: Uuid) -> Result<Option<AstridUserId>, IdentityError> {
        self.kv
            .get_json::<AstridUserId>(&Self::user_key(id))
            .await
            .map_err(|e| IdentityError::Storage(e.to_string()))
    }

    async fn resolve(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<AstridUserId>, IdentityError> {
        Self::validate_platform(platform)?;
        Self::validate_platform_user_id(platform_user_id)?;

        let normalized = normalize_platform(platform);
        let key = Self::link_key(&normalized, platform_user_id);

        let link: Option<FrontendLink> = self
            .kv
            .get_json(&key)
            .await
            .map_err(|e| IdentityError::Storage(e.to_string()))?;

        match link {
            Some(l) => self.get_user(l.astrid_user_id).await,
            None => Ok(None),
        }
    }

    async fn link(
        &self,
        platform: &str,
        platform_user_id: &str,
        astrid_user_id: Uuid,
        method: &str,
    ) -> Result<FrontendLink, IdentityError> {
        Self::validate_platform(platform)?;
        Self::validate_platform_user_id(platform_user_id)?;
        Self::validate_non_empty(method, "method")?;

        // Verify the target user exists.
        let user = self.get_user(astrid_user_id).await?;
        if user.is_none() {
            return Err(IdentityError::UserNotFound(astrid_user_id));
        }

        let normalized = normalize_platform(platform);
        let link = FrontendLink {
            platform: normalized.clone(),
            platform_user_id: platform_user_id.to_string(),
            astrid_user_id,
            linked_at: Utc::now(),
            method: method.to_string(),
        };

        let key = Self::link_key(&normalized, platform_user_id);
        self.kv
            .set_json(&key, &link)
            .await
            .map_err(|e| IdentityError::Storage(e.to_string()))?;

        Ok(link)
    }

    async fn unlink(&self, platform: &str, platform_user_id: &str) -> Result<bool, IdentityError> {
        Self::validate_platform(platform)?;
        Self::validate_platform_user_id(platform_user_id)?;

        let normalized = normalize_platform(platform);
        let key = Self::link_key(&normalized, platform_user_id);

        self.kv
            .delete(&key)
            .await
            .map_err(|e| IdentityError::Storage(e.to_string()))
    }

    async fn list_links(&self, astrid_user_id: Uuid) -> Result<Vec<FrontendLink>, IdentityError> {
        let keys = self
            .kv
            .list_keys_with_prefix("link/")
            .await
            .map_err(|e| IdentityError::Storage(e.to_string()))?;

        let mut links = Vec::new();
        for key in keys {
            if let Some(link) = self
                .kv
                .get_json::<FrontendLink>(&key)
                .await
                .map_err(|e| IdentityError::Storage(e.to_string()))?
                && link.astrid_user_id == astrid_user_id
            {
                links.push(link);
            }
        }
        Ok(links)
    }

    async fn get_user_by_name(&self, name: &str) -> Result<Option<AstridUserId>, IdentityError> {
        let key = Self::name_key(name.trim());
        let uuid_bytes = self
            .kv
            .get(&key)
            .await
            .map_err(|e| IdentityError::Storage(e.to_string()))?;

        match uuid_bytes {
            Some(bytes) => {
                let uuid_str = String::from_utf8(bytes)
                    .map_err(|e| IdentityError::Storage(format!("invalid UUID bytes: {e}")))?;
                let id = Uuid::parse_str(&uuid_str)
                    .map_err(|e| IdentityError::Storage(format!("invalid UUID: {e}")))?;
                self.get_user(id).await
            },
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::MemoryKvStore;

    fn make_store() -> KvIdentityStore {
        let kv_backend = Arc::new(MemoryKvStore::new());
        let scoped = ScopedKvStore::new(kv_backend, "system:identity").unwrap();
        KvIdentityStore::new(scoped)
    }

    #[tokio::test]
    async fn create_and_get_user() {
        let store = make_store();

        let user = store.create_user(Some("Alice")).await.unwrap();
        assert_eq!(user.display_name.as_deref(), Some("Alice"));

        let fetched = store.get_user(user.id).await.unwrap();
        assert_eq!(fetched, Some(user));
    }

    #[tokio::test]
    async fn create_user_no_name() {
        let store = make_store();
        let user = store.create_user(None).await.unwrap();
        assert!(user.display_name.is_none());
    }

    #[tokio::test]
    async fn get_nonexistent_user() {
        let store = make_store();
        let result = store.get_user(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn resolve_linked_user() {
        let store = make_store();
        let user = store.create_user(Some("Bob")).await.unwrap();

        store
            .link("Discord", "12345", user.id, "admin")
            .await
            .unwrap();

        let resolved = store.resolve("discord", "12345").await.unwrap();
        assert_eq!(resolved.unwrap().id, user.id);
    }

    #[tokio::test]
    async fn resolve_unlinked_returns_none() {
        let store = make_store();
        let result = store.resolve("discord", "99999").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn resolve_normalizes_platform() {
        let store = make_store();
        let user = store.create_user(None).await.unwrap();

        store
            .link("  DISCORD  ", "abc", user.id, "admin")
            .await
            .unwrap();

        // Different casing/whitespace should still resolve.
        let resolved = store.resolve("Discord", "abc").await.unwrap();
        assert_eq!(resolved.unwrap().id, user.id);
    }

    #[tokio::test]
    async fn link_requires_existing_user() {
        let store = make_store();
        let fake_id = Uuid::new_v4();

        let err = store
            .link("discord", "123", fake_id, "admin")
            .await
            .unwrap_err();
        assert!(matches!(err, IdentityError::UserNotFound(_)));
    }

    #[tokio::test]
    async fn link_upsert_semantics() {
        let store = make_store();
        let user1 = store.create_user(Some("Alice")).await.unwrap();
        let user2 = store.create_user(Some("Bob")).await.unwrap();

        store
            .link("discord", "123", user1.id, "admin")
            .await
            .unwrap();
        // Re-link to a different user (upsert).
        store
            .link("discord", "123", user2.id, "admin")
            .await
            .unwrap();

        let resolved = store.resolve("discord", "123").await.unwrap();
        assert_eq!(resolved.unwrap().id, user2.id);
    }

    #[tokio::test]
    async fn unlink_removes_link() {
        let store = make_store();
        let user = store.create_user(None).await.unwrap();

        store
            .link("telegram", "789", user.id, "admin")
            .await
            .unwrap();
        let removed = store.unlink("telegram", "789").await.unwrap();
        assert!(removed);

        let resolved = store.resolve("telegram", "789").await.unwrap();
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn unlink_nonexistent_returns_false() {
        let store = make_store();
        let removed = store.unlink("discord", "nonexistent").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn list_links_filters_by_user() {
        let store = make_store();
        let alice = store.create_user(Some("Alice")).await.unwrap();
        let bob = store.create_user(Some("Bob")).await.unwrap();

        store
            .link("discord", "a1", alice.id, "admin")
            .await
            .unwrap();
        store
            .link("telegram", "a2", alice.id, "admin")
            .await
            .unwrap();
        store.link("discord", "b1", bob.id, "admin").await.unwrap();

        let alice_links = store.list_links(alice.id).await.unwrap();
        assert_eq!(alice_links.len(), 2);
        assert!(alice_links.iter().all(|l| l.astrid_user_id == alice.id));

        let bob_links = store.list_links(bob.id).await.unwrap();
        assert_eq!(bob_links.len(), 1);
    }

    #[tokio::test]
    async fn list_links_empty_for_unknown_user() {
        let store = make_store();
        let links = store.list_links(Uuid::new_v4()).await.unwrap();
        assert!(links.is_empty());
    }

    #[tokio::test]
    async fn get_user_by_name_works() {
        let store = make_store();
        let user = store.create_user(Some("Charlie")).await.unwrap();

        let found = store.get_user_by_name("Charlie").await.unwrap();
        assert_eq!(found.unwrap().id, user.id);
    }

    #[tokio::test]
    async fn get_user_by_name_not_found() {
        let store = make_store();
        let found = store.get_user_by_name("nobody").await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn empty_platform_rejected() {
        let store = make_store();
        let err = store.resolve("", "123").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn empty_platform_user_id_rejected() {
        let store = make_store();
        let err = store.resolve("discord", "  ").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn link_empty_method_rejected() {
        let store = make_store();
        let user = store.create_user(None).await.unwrap();
        let err = store.link("discord", "123", user.id, "").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn platform_user_id_with_slash_rejected() {
        let store = make_store();
        let user = store.create_user(None).await.unwrap();

        // link rejects slash
        let err = store
            .link("discord", "123/../../user/456", user.id, "admin")
            .await
            .unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));

        // resolve rejects slash
        let err = store.resolve("discord", "a/b").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));

        // unlink rejects slash
        let err = store.unlink("discord", "x/y").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn platform_user_id_with_null_rejected() {
        let store = make_store();
        let err = store.resolve("discord", "abc\0def").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn platform_with_slash_rejected() {
        let store = make_store();
        let user = store.create_user(None).await.unwrap();

        let err = store
            .link("a/b", "123", user.id, "admin")
            .await
            .unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));

        let err = store.resolve("x/y", "123").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));

        let err = store.unlink("m/n", "123").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn platform_with_null_rejected() {
        let store = make_store();
        let err = store.resolve("disc\0rd", "123").await.unwrap_err();
        assert!(matches!(err, IdentityError::InvalidInput(_)));
    }
}
