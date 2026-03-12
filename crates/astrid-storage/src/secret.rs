//! Secure secret storage abstraction for capsule credentials.
//!
//! Provides a [`SecretStore`] trait with two implementations:
//!
//! - **[`KeychainSecretStore`]** (behind the `keychain` feature): Uses the OS
//!   keychain (macOS Keychain, Linux secret-service) via the `keyring` crate.
//! - **[`KvSecretStore`]**: Falls back to the existing [`ScopedKvStore`] with a
//!   `__secret:` key prefix. Suitable for headless/CI environments.
//!
//! Production code should use [`FallbackSecretStore`], which tries the keychain
//! first and degrades to KV storage when the keychain is unavailable.

use std::fmt;
use std::sync::Arc;

use crate::kv::ScopedKvStore;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from secret storage operations.
#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    /// The platform keychain is not accessible (headless, locked, no daemon).
    #[error("keychain not accessible: {0}")]
    NoAccess(String),

    /// The key or value was invalid for the backend.
    #[error("invalid secret key or value: {0}")]
    Invalid(String),

    /// An internal or platform error occurred.
    #[error("secret store error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Secure secret storage for capsule credentials.
///
/// Implementations must be `Send + Sync` for use in WASM host function
/// `UserData<HostState>`. All methods are synchronous because they are called
/// from synchronous Extism host functions that bridge to async via
/// `runtime_handle.block_on()`.
pub trait SecretStore: Send + Sync + fmt::Debug {
    /// Store a secret value for the given key.
    ///
    /// # Errors
    ///
    /// Returns an error if the key is empty, the backend rejects the value,
    /// or a platform error occurs.
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError>;

    /// Check whether a secret exists for the given key.
    ///
    /// # Errors
    ///
    /// Returns an error if the key is empty or a platform error occurs.
    fn exists(&self, key: &str) -> Result<bool, SecretStoreError>;

    /// Retrieve a secret value. Returns `None` if not found.
    ///
    /// # Errors
    ///
    /// Returns an error if the key is empty or a platform error occurs.
    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError>;

    /// Delete a secret. Returns `true` if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the key is empty or a platform error occurs.
    fn delete(&self, key: &str) -> Result<bool, SecretStoreError>;
}

/// Validate that a secret key is non-empty.
fn validate_key(key: &str) -> Result<(), SecretStoreError> {
    if key.is_empty() {
        return Err(SecretStoreError::Invalid(
            "secret key must not be empty".into(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// KV-backed implementation (always available)
// ---------------------------------------------------------------------------

/// KV-backed secret store using the `__secret:` key prefix convention.
///
/// This is the fallback for environments where the OS keychain is unavailable
/// (CI, headless servers, containers). Secrets are stored in the same
/// [`ScopedKvStore`] as other plugin data, namespaced to
/// `plugin:{capsule_id}:__secret:{key}`.
///
/// Less secure than the OS keychain (secrets at rest in the KV database
/// without OS-level encryption) but functional everywhere.
pub struct KvSecretStore {
    kv: ScopedKvStore,
    runtime_handle: tokio::runtime::Handle,
}

impl fmt::Debug for KvSecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KvSecretStore")
            .field("namespace", &self.kv.namespace())
            .finish_non_exhaustive()
    }
}

impl KvSecretStore {
    /// Create a new KV-backed secret store.
    #[must_use]
    pub fn new(kv: ScopedKvStore, runtime_handle: tokio::runtime::Handle) -> Self {
        Self { kv, runtime_handle }
    }

    /// The prefixed key used in the underlying KV store.
    fn prefixed_key(key: &str) -> String {
        format!("__secret:{key}")
    }
}

impl SecretStore for KvSecretStore {
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        validate_key(key)?;
        let prefixed = Self::prefixed_key(key);
        self.runtime_handle
            .block_on(self.kv.set(&prefixed, value.as_bytes().to_vec()))
            .map_err(|e| SecretStoreError::Internal(format!("KV set failed: {e}")))
    }

    fn exists(&self, key: &str) -> Result<bool, SecretStoreError> {
        validate_key(key)?;
        let prefixed = Self::prefixed_key(key);
        self.runtime_handle
            .block_on(self.kv.exists(&prefixed))
            .map_err(|e| SecretStoreError::Internal(format!("KV exists failed: {e}")))
    }

    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        validate_key(key)?;
        let prefixed = Self::prefixed_key(key);
        let bytes = self
            .runtime_handle
            .block_on(self.kv.get(&prefixed))
            .map_err(|e| SecretStoreError::Internal(format!("KV get failed: {e}")))?;
        match bytes {
            Some(b) => {
                let s = String::from_utf8(b)
                    .map_err(|e| SecretStoreError::Internal(format!("bad UTF-8 in secret: {e}")))?;
                Ok(Some(s))
            },
            None => Ok(None),
        }
    }

    fn delete(&self, key: &str) -> Result<bool, SecretStoreError> {
        validate_key(key)?;
        let prefixed = Self::prefixed_key(key);
        self.runtime_handle
            .block_on(self.kv.delete(&prefixed))
            .map_err(|e| SecretStoreError::Internal(format!("KV delete failed: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Keychain-backed implementation (behind `keychain` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "keychain")]
mod keychain_impl {
    use super::{SecretStore, SecretStoreError, validate_key};

    /// OS keychain-backed secret store using the `keyring` crate.
    ///
    /// Each secret is stored as a keyring entry with:
    /// - **service**: `"astrid:{capsule_id}"`
    /// - **user**: the secret key name (e.g. `"api_key"`)
    ///
    /// This provides per-capsule isolation at the OS level. Different capsules
    /// use different service names and cannot read each other's secrets.
    #[derive(Debug)]
    pub struct KeychainSecretStore {
        /// The keyring service name, typically `"astrid:{capsule_id}"`.
        service: String,
    }

    impl KeychainSecretStore {
        /// Create a new keychain-backed secret store for a capsule.
        ///
        /// The `capsule_id` is used to scope all secrets under the service
        /// name `"astrid:{capsule_id}"`.
        #[must_use]
        pub fn new(capsule_id: &str) -> Self {
            Self {
                service: format!("astrid:{capsule_id}"),
            }
        }

        /// Build a keyring `Entry` for the given key.
        fn entry(&self, key: &str) -> Result<keyring::Entry, SecretStoreError> {
            keyring::Entry::new(&self.service, key).map_err(|e| match e {
                keyring::Error::Invalid(attr, reason) => {
                    SecretStoreError::Invalid(format!("{attr}: {reason}"))
                },
                keyring::Error::TooLong(attr, max) => {
                    SecretStoreError::Invalid(format!("{attr} exceeds max length {max}"))
                },
                other => SecretStoreError::Internal(other.to_string()),
            })
        }
    }

    /// Map a keyring error to a `SecretStoreError`, treating `NoEntry` as a
    /// non-error condition (returns the provided default instead).
    fn map_keyring_error(e: keyring::Error) -> SecretStoreError {
        match e {
            keyring::Error::NoStorageAccess(inner) => SecretStoreError::NoAccess(inner.to_string()),
            keyring::Error::PlatformFailure(inner) => {
                SecretStoreError::Internal(format!("platform failure: {inner}"))
            },
            keyring::Error::Invalid(attr, reason) => {
                SecretStoreError::Invalid(format!("{attr}: {reason}"))
            },
            keyring::Error::TooLong(attr, max) => {
                SecretStoreError::Invalid(format!("{attr} exceeds max length {max}"))
            },
            keyring::Error::BadEncoding(bytes) => {
                SecretStoreError::Internal(format!("bad encoding: {} bytes", bytes.len()))
            },
            keyring::Error::Ambiguous(entries) => SecretStoreError::Internal(format!(
                "ambiguous: {} matching credentials",
                entries.len()
            )),
            // NoEntry is handled by callers, not mapped here
            keyring::Error::NoEntry => SecretStoreError::Internal("unexpected NoEntry".into()),
            // keyring::Error is #[non_exhaustive]
            other => SecretStoreError::Internal(other.to_string()),
        }
    }

    impl SecretStore for KeychainSecretStore {
        fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
            validate_key(key)?;
            let entry = self.entry(key)?;
            entry.set_password(value).map_err(map_keyring_error)
        }

        fn exists(&self, key: &str) -> Result<bool, SecretStoreError> {
            validate_key(key)?;
            let entry = self.entry(key)?;
            match entry.get_password() {
                Ok(_) => Ok(true),
                Err(keyring::Error::NoEntry) => Ok(false),
                Err(e) => Err(map_keyring_error(e)),
            }
        }

        fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
            validate_key(key)?;
            let entry = self.entry(key)?;
            match entry.get_password() {
                Ok(password) => Ok(Some(password)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(map_keyring_error(e)),
            }
        }

        fn delete(&self, key: &str) -> Result<bool, SecretStoreError> {
            validate_key(key)?;
            let entry = self.entry(key)?;
            match entry.delete_credential() {
                Ok(()) => Ok(true),
                Err(keyring::Error::NoEntry) => Ok(false),
                Err(e) => Err(map_keyring_error(e)),
            }
        }
    }
}

#[cfg(feature = "keychain")]
pub use keychain_impl::KeychainSecretStore;

// ---------------------------------------------------------------------------
// Fallback: keychain with KV degradation
// ---------------------------------------------------------------------------

#[cfg(feature = "keychain")]
mod fallback_impl {
    use std::fmt;

    use super::{KeychainSecretStore, KvSecretStore, SecretStore, SecretStoreError};

    /// Composite secret store that tries the OS keychain first and falls back
    /// to KV storage when the keychain is unavailable.
    ///
    /// On the first `NoAccess` error from the keychain, a warning is logged.
    /// Subsequent operations continue to try the keychain (it may become
    /// available if the user unlocks it).
    pub struct FallbackSecretStore {
        keychain: KeychainSecretStore,
        kv: KvSecretStore,
    }

    impl fmt::Debug for FallbackSecretStore {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("FallbackSecretStore")
                .field("keychain", &self.keychain)
                .field("kv", &self.kv)
                .finish()
        }
    }

    impl FallbackSecretStore {
        /// Create a new fallback secret store.
        #[must_use]
        pub fn new(keychain: KeychainSecretStore, kv: KvSecretStore) -> Self {
            Self { keychain, kv }
        }
    }

    impl SecretStore for FallbackSecretStore {
        fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
            match self.keychain.set(key, value) {
                Ok(()) => Ok(()),
                Err(SecretStoreError::NoAccess(reason)) => {
                    tracing::warn!(
                        %key,
                        %reason,
                        "OS keychain unavailable, falling back to KV secret storage"
                    );
                    self.kv.set(key, value)
                },
                Err(e) => Err(e),
            }
        }

        fn exists(&self, key: &str) -> Result<bool, SecretStoreError> {
            match self.keychain.exists(key) {
                Ok(exists) => Ok(exists),
                Err(SecretStoreError::NoAccess(reason)) => {
                    tracing::warn!(
                        %key,
                        %reason,
                        "OS keychain unavailable, falling back to KV secret storage"
                    );
                    self.kv.exists(key)
                },
                Err(e) => Err(e),
            }
        }

        fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
            match self.keychain.get(key) {
                Ok(val) => Ok(val),
                Err(SecretStoreError::NoAccess(reason)) => {
                    tracing::warn!(
                        %key,
                        %reason,
                        "OS keychain unavailable, falling back to KV secret storage"
                    );
                    self.kv.get(key)
                },
                Err(e) => Err(e),
            }
        }

        fn delete(&self, key: &str) -> Result<bool, SecretStoreError> {
            match self.keychain.delete(key) {
                Ok(deleted) => Ok(deleted),
                Err(SecretStoreError::NoAccess(reason)) => {
                    tracing::warn!(
                        %key,
                        %reason,
                        "OS keychain unavailable, falling back to KV secret storage"
                    );
                    self.kv.delete(key)
                },
                Err(e) => Err(e),
            }
        }
    }
}

#[cfg(feature = "keychain")]
pub use fallback_impl::FallbackSecretStore;

// ---------------------------------------------------------------------------
// Convenience constructor
// ---------------------------------------------------------------------------

/// Create the best available [`SecretStore`] for production use.
///
/// With the `keychain` feature enabled, returns a [`FallbackSecretStore`] that
/// tries the OS keychain first. Without the feature, returns a [`KvSecretStore`].
#[must_use]
pub fn build_secret_store(
    capsule_id: &str,
    kv: ScopedKvStore,
    runtime_handle: tokio::runtime::Handle,
) -> Arc<dyn SecretStore> {
    let _ = capsule_id; // used only with keychain feature
    let kv_store = KvSecretStore::new(kv, runtime_handle);

    #[cfg(feature = "keychain")]
    {
        let keychain = KeychainSecretStore::new(capsule_id);
        Arc::new(FallbackSecretStore::new(keychain, kv_store))
    }

    #[cfg(not(feature = "keychain"))]
    {
        Arc::new(kv_store)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{KvSecretStore, ScopedKvStore, SecretStore, SecretStoreError, build_secret_store};
    use crate::MemoryKvStore;

    /// Build a `KvSecretStore` backed by an in-memory KV. Returns a dedicated
    /// tokio runtime that the store uses internally for `block_on`.
    fn make_kv_store() -> (KvSecretStore, ScopedKvStore, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let store = Arc::new(MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "plugin:test-capsule").unwrap();
        let secret_store = KvSecretStore::new(kv.clone(), rt.handle().clone());
        (secret_store, kv, rt)
    }

    #[test]
    fn kv_set_and_exists() {
        let (store, _kv, _rt) = make_kv_store();
        assert!(!store.exists("api_key").unwrap());
        store.set("api_key", "sk-12345").unwrap();
        assert!(store.exists("api_key").unwrap());
    }

    #[test]
    fn kv_set_and_get() {
        let (store, _kv, _rt) = make_kv_store();
        assert_eq!(store.get("api_key").unwrap(), None);
        store.set("api_key", "sk-12345").unwrap();
        assert_eq!(store.get("api_key").unwrap(), Some("sk-12345".into()));
    }

    #[test]
    fn kv_delete_existing() {
        let (store, _kv, _rt) = make_kv_store();
        store.set("api_key", "sk-12345").unwrap();
        assert!(store.delete("api_key").unwrap());
        assert!(!store.exists("api_key").unwrap());
    }

    #[test]
    fn kv_delete_nonexistent() {
        let (store, _kv, _rt) = make_kv_store();
        assert!(!store.delete("missing").unwrap());
    }

    #[test]
    fn kv_empty_key_rejected() {
        let (store, _kv, _rt) = make_kv_store();
        assert!(matches!(
            store.set("", "value"),
            Err(SecretStoreError::Invalid(_))
        ));
        assert!(matches!(
            store.exists(""),
            Err(SecretStoreError::Invalid(_))
        ));
        assert!(matches!(store.get(""), Err(SecretStoreError::Invalid(_))));
        assert!(matches!(
            store.delete(""),
            Err(SecretStoreError::Invalid(_))
        ));
    }

    #[test]
    fn kv_overwrite_secret() {
        let (store, _kv, _rt) = make_kv_store();
        store.set("key", "v1").unwrap();
        store.set("key", "v2").unwrap();
        assert_eq!(store.get("key").unwrap(), Some("v2".into()));
    }

    #[test]
    fn kv_isolation_between_keys() {
        let (store, _kv, _rt) = make_kv_store();
        store.set("key_a", "a").unwrap();
        store.set("key_b", "b").unwrap();
        assert_eq!(store.get("key_a").unwrap(), Some("a".into()));
        assert_eq!(store.get("key_b").unwrap(), Some("b".into()));
        assert!(!store.exists("key_c").unwrap());
    }

    #[test]
    fn kv_prefixed_key_format() {
        let (store, kv, rt) = make_kv_store();
        store.set("my_secret", "value").unwrap();
        // Verify the underlying KV uses the __secret: prefix
        let raw = rt.block_on(kv.get("__secret:my_secret")).unwrap();
        assert_eq!(raw, Some(b"value".to_vec()));
    }

    #[test]
    fn build_secret_store_returns_arc() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let store = Arc::new(MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "plugin:test").unwrap();
        let secret_store = build_secret_store("test", kv, rt.handle().clone());
        assert!(!secret_store.exists("nonexistent").unwrap());
    }
}
