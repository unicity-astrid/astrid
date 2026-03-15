//! Capability token storage.
//!
//! Provides both in-memory (session) and persistent (`SurrealKV`) storage
//! for capability tokens.

use astrid_core::{Permission, TokenId};
use astrid_storage::{KvStore, SurrealKvStore};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::error::{CapabilityError, CapabilityResult};
use crate::token::CapabilityToken;

// -- Namespace constants --

const NS_TOKENS: &str = "caps:tokens";
const NS_REVOKED: &str = "caps:revoked";
const NS_USED: &str = "caps:used";

/// Tombstone value for presence-only KV entries (revoked/used markers).
const PRESENCE_MARKER: &[u8] = &[1];

/// Run an async future synchronously.
///
/// Handles three cases:
/// - Inside an async context: uses a scoped thread to avoid the
///   "cannot `block_on` from within a runtime" panic.
/// - Outside a runtime: creates a temporary runtime.
fn block_on<F>(f: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => std::thread::scope(|s| {
            s.spawn(|| handle.block_on(f))
                .join()
                .expect("async thread panicked")
        }),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
            .block_on(f),
    }
}

/// Capability store with both session and persistent storage.
pub struct CapabilityStore {
    /// Session tokens (in-memory, cleared on session end).
    session_tokens: RwLock<HashMap<TokenId, CapabilityToken>>,
    /// Persistent tokens (`KvStore` backed).
    persistent_store: Option<Arc<dyn KvStore>>,
    /// Revoked token IDs (quick lookup).
    revoked: RwLock<std::collections::HashSet<TokenId>>,
    /// Used single-use token IDs (replay protection).
    used_tokens: RwLock<std::collections::HashSet<TokenId>>,
}

impl CapabilityStore {
    /// Create an in-memory only store (no persistence).
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            session_tokens: RwLock::new(HashMap::new()),
            persistent_store: None,
            revoked: RwLock::new(std::collections::HashSet::new()),
            used_tokens: RwLock::new(std::collections::HashSet::new()),
        }
    }

    /// Create a store with persistence.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or read.
    pub fn with_persistence(path: impl AsRef<Path>) -> CapabilityResult<Self> {
        let store =
            SurrealKvStore::open(path).map_err(|e| CapabilityError::StorageError(e.to_string()))?;
        let kv: Arc<dyn KvStore> = Arc::new(store);

        let mut cap_store = Self {
            session_tokens: RwLock::new(HashMap::new()),
            persistent_store: Some(kv),
            revoked: RwLock::new(std::collections::HashSet::new()),
            used_tokens: RwLock::new(std::collections::HashSet::new()),
        };

        // Load revoked and used tokens
        cap_store.load_revoked()?;
        cap_store.load_used_tokens()?;

        Ok(cap_store)
    }

    /// Create a store backed by an existing `KvStore` (for shared stores).
    ///
    /// # Errors
    ///
    /// Returns an error if loading existing revoked/used tokens fails.
    pub fn with_kv_store(store: Arc<dyn KvStore>) -> CapabilityResult<Self> {
        let mut cap_store = Self {
            session_tokens: RwLock::new(HashMap::new()),
            persistent_store: Some(store),
            revoked: RwLock::new(std::collections::HashSet::new()),
            used_tokens: RwLock::new(std::collections::HashSet::new()),
        };

        cap_store.load_revoked()?;
        cap_store.load_used_tokens()?;

        Ok(cap_store)
    }

    /// Load revoked token IDs from persistent storage.
    fn load_revoked(&mut self) -> CapabilityResult<()> {
        let Some(store) = &self.persistent_store else {
            return Ok(());
        };

        let keys = block_on(store.list_keys(NS_REVOKED))
            .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

        let mut revoked = self
            .revoked
            .write()
            .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

        for key in keys {
            if let Ok(uuid) = uuid::Uuid::parse_str(&key) {
                revoked.insert(TokenId::from_uuid(uuid));
            }
        }

        Ok(())
    }

    /// Load used single-use token IDs from persistent storage.
    fn load_used_tokens(&mut self) -> CapabilityResult<()> {
        let Some(store) = &self.persistent_store else {
            return Ok(());
        };

        let keys = block_on(store.list_keys(NS_USED))
            .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

        let mut used = self
            .used_tokens
            .write()
            .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

        for key in keys {
            if let Ok(uuid) = uuid::Uuid::parse_str(&key) {
                used.insert(TokenId::from_uuid(uuid));
            }
        }

        Ok(())
    }

    /// Add a capability token.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is invalid or storage fails.
    pub fn add(&self, token: CapabilityToken) -> CapabilityResult<()> {
        // Validate the token first
        token.validate()?;

        match token.scope {
            crate::token::TokenScope::Session => {
                let mut tokens = self
                    .session_tokens
                    .write()
                    .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
                tokens.insert(token.id.clone(), token);
            },
            crate::token::TokenScope::Persistent => {
                if let Some(store) = &self.persistent_store {
                    let serialized = serde_json::to_vec(&token)
                        .map_err(|e| CapabilityError::SerializationError(e.to_string()))?;

                    let key = token.id.0.to_string();
                    block_on(store.set(NS_TOKENS, &key, serialized))
                        .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
                } else {
                    // Fall back to session storage if no persistence
                    let mut tokens = self
                        .session_tokens
                        .write()
                        .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
                    tokens.insert(token.id.clone(), token);
                }
            },
        }

        Ok(())
    }

    /// Get a token by ID.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::TokenRevoked`] if the token has been revoked,
    /// or a storage error if reading fails.
    pub fn get(&self, token_id: &TokenId) -> CapabilityResult<Option<CapabilityToken>> {
        // Check if revoked
        {
            let revoked = self
                .revoked
                .read()
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
            if revoked.contains(token_id) {
                return Err(CapabilityError::TokenRevoked {
                    token_id: token_id.to_string(),
                });
            }
        }

        // Check session tokens first
        {
            let tokens = self
                .session_tokens
                .read()
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
            if let Some(token) = tokens.get(token_id) {
                return Ok(Some(token.clone()));
            }
        }

        // Check persistent storage
        if let Some(store) = &self.persistent_store {
            let key = token_id.0.to_string();
            let data = block_on(store.get(NS_TOKENS, &key))
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

            if let Some(bytes) = data {
                let token: CapabilityToken = serde_json::from_slice(&bytes)
                    .map_err(|e| CapabilityError::SerializationError(e.to_string()))?;
                // Defense in depth: re-verify persistent tokens on read.
                // Session tokens were validated at add() time and live in
                // trusted memory, but persistent tokens could be tampered on
                // disk.
                token.validate()?;
                return Ok(Some(token));
            }
        }

        Ok(None)
    }

    /// Check if a single-use token has already been consumed.
    ///
    /// Returns `Ok(true)` if the token is single-use and already consumed.
    /// Returns `Ok(false)` if the token is not single-use or has not been used.
    /// Returns `Err(())` on lock poisoning, to support fail-closed callers.
    fn is_consumed_single_use(&self, token: &CapabilityToken) -> Result<bool, ()> {
        if !token.is_single_use() {
            return Ok(false);
        }
        let used = self.used_tokens.read().map_err(|_| ())?;
        Ok(used.contains(&token.id))
    }

    /// Check if there's a capability for a resource and permission.
    pub fn has_capability(&self, resource: &str, permission: Permission) -> bool {
        // Check session tokens
        if let Ok(tokens) = self.session_tokens.read() {
            for token in tokens.values() {
                if !token.is_expired() && token.grants(resource, permission) {
                    match self.is_consumed_single_use(token) {
                        Ok(true) => {},
                        Ok(false) => return true,
                        Err(()) => return false,
                    }
                }
            }
        }

        // Check persistent tokens
        if let Some(store) = &self.persistent_store
            && let Ok(keys) = block_on(store.list_keys(NS_TOKENS))
        {
            for key in keys {
                if let Ok(Some(data)) = block_on(store.get(NS_TOKENS, &key))
                    && let Ok(token) = serde_json::from_slice::<CapabilityToken>(&data)
                {
                    // Defense in depth: validate persistent tokens (expiry +
                    // signature). Uses validate() for consistency with get()
                    // so future checks (e.g. nbf) are applied uniformly.
                    if let Err(e) = token.validate() {
                        if matches!(e, CapabilityError::TokenExpired { .. }) {
                            tracing::debug!(token_id = %token.id, "skipping expired persistent token");
                        } else {
                            tracing::warn!(token_id = %token.id, "skipping invalid persistent token: {e}");
                        }
                        continue;
                    }
                    // Check if not revoked
                    if let Ok(revoked) = self.revoked.read()
                        && revoked.contains(&token.id)
                    {
                        continue;
                    }
                    if token.grants(resource, permission) {
                        match self.is_consumed_single_use(&token) {
                            Ok(true) => {},
                            Ok(false) => return true,
                            Err(()) => return false,
                        }
                    }
                }
            }
        }

        false
    }

    /// Find a token that grants a capability.
    pub fn find_capability(
        &self,
        resource: &str,
        permission: Permission,
    ) -> Option<CapabilityToken> {
        // Check session tokens
        if let Ok(tokens) = self.session_tokens.read() {
            for token in tokens.values() {
                if !token.is_expired() && token.grants(resource, permission) {
                    match self.is_consumed_single_use(token) {
                        Ok(true) => {},
                        Ok(false) => return Some(token.clone()),
                        Err(()) => return None,
                    }
                }
            }
        }

        // Check persistent tokens
        if let Some(store) = &self.persistent_store
            && let Ok(keys) = block_on(store.list_keys(NS_TOKENS))
        {
            for key in keys {
                if let Ok(Some(data)) = block_on(store.get(NS_TOKENS, &key))
                    && let Ok(token) = serde_json::from_slice::<CapabilityToken>(&data)
                {
                    // Defense in depth: validate persistent tokens (expiry +
                    // signature). Uses validate() for consistency with get().
                    if let Err(e) = token.validate() {
                        if matches!(e, CapabilityError::TokenExpired { .. }) {
                            tracing::debug!(token_id = %token.id, "skipping expired persistent token");
                        } else {
                            tracing::warn!(token_id = %token.id, "skipping invalid persistent token: {e}");
                        }
                        continue;
                    }
                    // Check if not revoked
                    if let Ok(revoked) = self.revoked.read()
                        && revoked.contains(&token.id)
                    {
                        continue;
                    }
                    if token.grants(resource, permission) {
                        match self.is_consumed_single_use(&token) {
                            Ok(true) => {},
                            Ok(false) => return Some(token),
                            Err(()) => return None,
                        }
                    }
                }
            }
        }

        None
    }

    /// Revoke a token.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn revoke(&self, token_id: &TokenId) -> CapabilityResult<()> {
        // Persist revocation first so KV is the ground truth. If the daemon
        // crashes after this point, `load_revoked()` will still see it on
        // restart.
        if let Some(store) = &self.persistent_store {
            let key = token_id.0.to_string();

            block_on(store.set(NS_REVOKED, &key, PRESENCE_MARKER.to_vec()))
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

            if let Err(e) = block_on(store.delete(NS_TOKENS, &key)) {
                tracing::warn!("failed to delete revoked token from caps:tokens: {e}");
            }
        }

        // Update in-memory state (rebuilt from KV on restart regardless).
        {
            let mut revoked = self
                .revoked
                .write()
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
            revoked.insert(token_id.clone());
        }

        {
            let mut tokens = self
                .session_tokens
                .write()
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
            tokens.remove(token_id);
        }

        Ok(())
    }

    /// Clear all session tokens.
    ///
    /// # Errors
    ///
    /// Returns an error if the lock cannot be acquired.
    pub fn clear_session(&self) -> CapabilityResult<()> {
        let mut tokens = self
            .session_tokens
            .write()
            .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
        tokens.clear();
        Ok(())
    }

    /// Mark a single-use token as used.
    ///
    /// This should be called after successfully using a single-use token
    /// to prevent replay attacks.
    ///
    /// # Errors
    ///
    /// Returns an error if the token was already used or storage fails.
    pub fn mark_used(&self, token_id: &TokenId) -> CapabilityResult<()> {
        // Hold a single write lock across check, persist, and insert to
        // prevent TOCTOU races where two concurrent callers both pass
        // the "already used?" check before either inserts.
        let mut used = self
            .used_tokens
            .write()
            .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

        if used.contains(token_id) {
            return Err(CapabilityError::TokenAlreadyUsed {
                token_id: token_id.to_string(),
            });
        }

        // Persist first so KV is the ground truth. If the daemon crashes
        // after this point, `load_used_tokens()` will still see it on
        // restart. Holding the write lock across `block_on` is safe
        // because `block_on` spawns an OS thread and does not re-acquire
        // any lock on this store.
        if let Some(store) = &self.persistent_store {
            block_on(store.set(NS_USED, &token_id.0.to_string(), PRESENCE_MARKER.to_vec()))
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
        }

        used.insert(token_id.clone());
        Ok(())
    }

    /// Check if a single-use token has been used.
    pub fn is_used(&self, token_id: &TokenId) -> bool {
        self.used_tokens
            .read()
            .map(|used| used.contains(token_id))
            .unwrap_or(false)
    }

    /// Validate and optionally consume a token.
    ///
    /// For single-use tokens, this marks them as used.
    /// For regular tokens, this just validates them.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is invalid, expired, revoked, or already used.
    pub fn use_token(&self, token_id: &TokenId) -> CapabilityResult<CapabilityToken> {
        let token = self
            .get(token_id)?
            .ok_or_else(|| CapabilityError::TokenNotFound {
                token_id: token_id.to_string(),
            })?;

        // Validate the token
        token.validate()?;

        // For single-use tokens, mark as used
        if token.is_single_use() {
            self.mark_used(token_id)?;
        }

        Ok(token)
    }

    /// List all valid tokens.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn list_tokens(&self) -> CapabilityResult<Vec<CapabilityToken>> {
        let mut tokens = Vec::new();

        // Session tokens
        {
            let session = self
                .session_tokens
                .read()
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
            for token in session.values() {
                if !token.is_expired() {
                    tokens.push(token.clone());
                }
            }
        }

        // Persistent tokens
        if let Some(store) = &self.persistent_store {
            let revoked = self
                .revoked
                .read()
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

            let keys = block_on(store.list_keys(NS_TOKENS))
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

            for key in keys {
                let data = block_on(store.get(NS_TOKENS, &key))
                    .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
                if let Some(bytes) = data
                    && let Ok(token) = serde_json::from_slice::<CapabilityToken>(&bytes)
                    && !revoked.contains(&token.id)
                    && !token.is_expired()
                {
                    tokens.push(token);
                }
            }
        }

        Ok(tokens)
    }

    /// Cleanup expired tokens from persistent storage.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn cleanup_expired(&self) -> CapabilityResult<usize> {
        let mut removed: usize = 0;

        if let Some(store) = &self.persistent_store {
            let keys = block_on(store.list_keys(NS_TOKENS))
                .map_err(|e| CapabilityError::StorageError(e.to_string()))?;

            for key in keys {
                let data = block_on(store.get(NS_TOKENS, &key))
                    .map_err(|e| CapabilityError::StorageError(e.to_string()))?;
                if let Some(bytes) = data
                    && let Ok(token) = serde_json::from_slice::<CapabilityToken>(&bytes)
                    && token.is_expired()
                {
                    let _ = block_on(store.delete(NS_TOKENS, &key));
                    removed = removed.saturating_add(1);
                }
            }
        }

        Ok(removed)
    }
}

impl Default for CapabilityStore {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl std::fmt::Debug for CapabilityStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let session_count = self.session_tokens.read().map(|t| t.len()).unwrap_or(0);
        let revoked_count = self.revoked.read().map(|r| r.len()).unwrap_or(0);
        let used_count = self.used_tokens.read().map(|u| u.len()).unwrap_or(0);
        let has_persistence = self.persistent_store.is_some();

        f.debug_struct("CapabilityStore")
            .field("session_tokens", &session_count)
            .field("revoked_count", &revoked_count)
            .field("used_count", &used_count)
            .field("has_persistence", &has_persistence)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::ResourcePattern;
    use crate::token::{AuditEntryId, TokenScope};
    use astrid_crypto::KeyPair;
    use astrid_storage::MemoryKvStore;

    fn test_keypair() -> KeyPair {
        KeyPair::generate()
    }

    #[tokio::test]
    async fn test_in_memory_store() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        let token_id = token.id.clone();

        store.add(token).unwrap();
        assert!(store.has_capability("mcp://test:tool", Permission::Invoke));
        assert!(store.get(&token_id).unwrap().is_some());
    }

    #[tokio::test]
    async fn test_revoke() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        let token_id = token.id.clone();

        store.add(token).unwrap();
        assert!(store.has_capability("mcp://test:tool", Permission::Invoke));

        store.revoke(&token_id).unwrap();
        assert!(!store.has_capability("mcp://test:tool", Permission::Invoke));
        assert!(matches!(
            store.get(&token_id),
            Err(CapabilityError::TokenRevoked { .. })
        ));
    }

    #[tokio::test]
    async fn test_clear_session() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        store.add(token).unwrap();
        assert!(store.has_capability("mcp://test:tool", Permission::Invoke));

        store.clear_session().unwrap();
        assert!(!store.has_capability("mcp://test:tool", Permission::Invoke));
    }

    #[tokio::test]
    async fn test_find_capability() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::new("mcp://filesystem:*").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        store.add(token).unwrap();

        let found = store.find_capability("mcp://filesystem:read_file", Permission::Invoke);
        assert!(found.is_some());

        let not_found = store.find_capability("mcp://memory:read", Permission::Invoke);
        assert!(not_found.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_persistent_store() {
        // Use an in-memory KvStore for testing (avoids filesystem issues).
        let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let store = CapabilityStore::with_kv_store(Arc::clone(&kv)).unwrap();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Persistent,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        let token_id = token.id.clone();

        store.add(token).unwrap();

        // Reload store to verify persistence (same backing store).
        drop(store);
        let store2 = CapabilityStore::with_kv_store(kv).unwrap();
        assert!(store2.get(&token_id).unwrap().is_some());
        // Verify find_capability (the production lookup path) also works after reload.
        assert!(
            store2
                .find_capability("mcp://test:tool", Permission::Invoke)
                .is_some()
        );

        // Also test disk-backed store can open and store/retrieve.
        // Note: SurrealKV holds an OS-level file lock, so we cannot drop-and-reopen
        // the same path in a single test. The in-memory `with_kv_store` test above
        // already validates the reload-from-backing-store pattern.
        let temp_dir = tempfile::tempdir().unwrap();
        let disk_store = CapabilityStore::with_persistence(temp_dir.path().join("caps")).unwrap();
        let token2 = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool2").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Persistent,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );
        let other_token_id = token2.id.clone();
        disk_store.add(token2).unwrap();
        assert!(disk_store.get(&other_token_id).unwrap().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_revocation_survives_restart() {
        let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let store = CapabilityStore::with_kv_store(Arc::clone(&kv)).unwrap();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Persistent,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        let token_id = token.id.clone();
        store.add(token).unwrap();
        store.revoke(&token_id).unwrap();

        // Reload - revocation must survive.
        drop(store);
        let store2 = CapabilityStore::with_kv_store(kv).unwrap();
        assert!(matches!(
            store2.get(&token_id),
            Err(CapabilityError::TokenRevoked { .. })
        ));
        assert!(!store2.has_capability("mcp://test:tool", Permission::Invoke));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_mark_used_survives_restart() {
        let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let store = CapabilityStore::with_kv_store(Arc::clone(&kv)).unwrap();
        let keypair = test_keypair();

        let token = CapabilityToken::create_with_options(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Persistent,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
            true,
        );

        let token_id = token.id.clone();
        store.add(token).unwrap();
        store.mark_used(&token_id).unwrap();

        // Reload - used state must survive.
        drop(store);
        let store2 = CapabilityStore::with_kv_store(kv).unwrap();
        assert!(store2.is_used(&token_id));
        assert!(!store2.has_capability("mcp://test:tool", Permission::Invoke));
    }

    #[tokio::test]
    async fn test_find_capability_excludes_used_single_use() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();

        let token = CapabilityToken::create_with_options(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
            true,
        );

        let token_id = token.id.clone();
        store.add(token).unwrap();

        // Before marking used: both find_capability and has_capability return the token
        assert!(
            store
                .find_capability("mcp://test:tool", Permission::Invoke)
                .is_some()
        );
        assert!(store.has_capability("mcp://test:tool", Permission::Invoke));

        // Mark the single-use token as consumed
        store.mark_used(&token_id).unwrap();

        // After marking used: both must exclude the consumed token
        assert!(
            store
                .find_capability("mcp://test:tool", Permission::Invoke)
                .is_none()
        );
        assert!(!store.has_capability("mcp://test:tool", Permission::Invoke));
    }

    /// Helper: create a valid persistent token, serialize it, tamper a field,
    /// and write the corrupted bytes directly to the KV store (bypassing
    /// `CapabilityStore::add` which validates). Returns the token ID.
    async fn inject_tampered_persistent_token(kv: &Arc<dyn KvStore>, keypair: &KeyPair) -> TokenId {
        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://tampered:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Persistent,
            keypair.key_id(),
            AuditEntryId::new(),
            keypair,
            None,
        );
        let token_id = token.id.clone();

        // Serialize, tamper a field (add an extra permission), re-serialize.
        // The signature was computed over the original data, so it will
        // no longer verify after tampering.
        let mut value: serde_json::Value = serde_json::to_value(&token).unwrap();
        value["permissions"] = serde_json::json!(["invoke", "read", "write"]);
        let tampered_bytes = serde_json::to_vec(&value).unwrap();

        kv.set(NS_TOKENS, &token_id.0.to_string(), tampered_bytes)
            .await
            .unwrap();
        token_id
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_rejects_tampered_persistent_token() {
        let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let store = CapabilityStore::with_kv_store(Arc::clone(&kv)).unwrap();
        let keypair = test_keypair();

        let token_id = inject_tampered_persistent_token(&kv, &keypair).await;

        // get() should return an error for tampered tokens
        let result = store.get(&token_id);
        assert!(
            matches!(result, Err(CapabilityError::InvalidSignature)),
            "expected InvalidSignature, got {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_find_capability_skips_tampered_persistent_token() {
        let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let store = CapabilityStore::with_kv_store(Arc::clone(&kv)).unwrap();
        let keypair = test_keypair();

        let _token_id = inject_tampered_persistent_token(&kv, &keypair).await;

        // find_capability should skip tampered tokens and return None
        assert!(
            store
                .find_capability("mcp://tampered:tool", Permission::Invoke)
                .is_none()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_has_capability_skips_tampered_persistent_token() {
        let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let store = CapabilityStore::with_kv_store(Arc::clone(&kv)).unwrap();
        let keypair = test_keypair();

        let _token_id = inject_tampered_persistent_token(&kv, &keypair).await;

        // has_capability should skip tampered tokens and return false
        assert!(!store.has_capability("mcp://tampered:tool", Permission::Invoke));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_find_capability_excludes_used_single_use_persistent() {
        let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let store = CapabilityStore::with_kv_store(Arc::clone(&kv)).unwrap();
        let keypair = test_keypair();

        let token = CapabilityToken::create_with_options(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Persistent,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
            true,
        );

        let token_id = token.id.clone();
        store.add(token).unwrap();

        assert!(
            store
                .find_capability("mcp://test:tool", Permission::Invoke)
                .is_some()
        );
        assert!(store.has_capability("mcp://test:tool", Permission::Invoke));

        store.mark_used(&token_id).unwrap();

        assert!(
            store
                .find_capability("mcp://test:tool", Permission::Invoke)
                .is_none()
        );
        assert!(!store.has_capability("mcp://test:tool", Permission::Invoke));
    }
}
