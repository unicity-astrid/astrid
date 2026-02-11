//! Raw key-value store trait and implementations.
//!
//! The [`KvStore`] trait provides byte-level `get`/`set`/`delete` operations
//! with namespaced keys. Implementations:
//!
//! - **In-memory** (always available): For tests and ephemeral data
//! - **`SurrealKV`** (behind `kv` feature): Persistent, versioned, ACID-compliant
//!
//! # Namespacing
//!
//! All operations are scoped to a namespace. WASM guests receive a namespace
//! like `wasm:{plugin_id}` and cannot access keys outside their namespace.
//! The runtime uses `system:*` namespaces for internal state.
//!
//! # Ergonomic Access
//!
//! Use [`ScopedKvStore`] to pre-bind a namespace. This is the primary API
//! for WASM guests — they receive a scoped store and never handle namespaces
//! directly. It also provides typed [`get_json`](ScopedKvStore::get_json) /
//! [`set_json`](ScopedKvStore::set_json) convenience methods.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{StorageError, StorageResult};

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate that a namespace is safe for use as a key prefix.
///
/// Namespaces must be non-empty and must not contain the null byte
/// (used internally as the namespace/key separator).
fn validate_namespace(namespace: &str) -> StorageResult<()> {
    if namespace.is_empty() {
        return Err(StorageError::InvalidKey(
            "namespace must not be empty".into(),
        ));
    }
    if namespace.contains('\0') {
        return Err(StorageError::InvalidKey(
            "namespace must not contain null bytes".into(),
        ));
    }
    Ok(())
}

/// Validate that a key is safe for storage.
///
/// Keys must be non-empty and must not contain the null byte.
fn validate_key(key: &str) -> StorageResult<()> {
    if key.is_empty() {
        return Err(StorageError::InvalidKey("key must not be empty".into()));
    }
    if key.contains('\0') {
        return Err(StorageError::InvalidKey(
            "key must not contain null bytes".into(),
        ));
    }
    Ok(())
}

/// Build the composite key `"{namespace}\0{key}"` as bytes.
#[cfg(feature = "kv")]
fn composite_key(namespace: &str, key: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(namespace.len() + 1 + key.len());
    buf.extend_from_slice(namespace.as_bytes());
    buf.push(0);
    buf.extend_from_slice(key.as_bytes());
    buf
}

/// Build the start of the namespace range (inclusive): `"{namespace}\0"`.
#[cfg(feature = "kv")]
fn namespace_range_start(namespace: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(namespace.len() + 1);
    buf.extend_from_slice(namespace.as_bytes());
    buf.push(0);
    buf
}

/// Build the end of the namespace range (exclusive): `"{namespace}\x01"`.
///
/// Since `\0` is the separator, any key in the namespace has the form
/// `"{namespace}\0{key}"`. The byte `\x01` immediately follows `\0`,
/// so the range `["{namespace}\0", "{namespace}\x01")` captures exactly
/// all keys in the namespace.
#[cfg(feature = "kv")]
fn namespace_range_end(namespace: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(namespace.len() + 1);
    buf.extend_from_slice(namespace.as_bytes());
    buf.push(1);
    buf
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A key-value entry with its namespace and key.
#[derive(Debug, Clone)]
pub struct KvEntry {
    /// The namespace this entry belongs to.
    pub namespace: String,
    /// The key within the namespace.
    pub key: String,
    /// The raw value bytes.
    pub value: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Raw key-value store trait.
///
/// Provides namespaced byte-level storage. All operations are scoped
/// to a namespace for isolation.
#[async_trait]
pub trait KvStore: Send + Sync {
    /// Get a value by namespace and key.
    ///
    /// Returns `None` if the key does not exist.
    async fn get(&self, namespace: &str, key: &str) -> StorageResult<Option<Vec<u8>>>;

    /// Set a value for a namespace and key.
    ///
    /// Overwrites any existing value.
    async fn set(&self, namespace: &str, key: &str, value: Vec<u8>) -> StorageResult<()>;

    /// Delete a key from a namespace.
    ///
    /// Returns `true` if the key existed and was deleted.
    async fn delete(&self, namespace: &str, key: &str) -> StorageResult<bool>;

    /// Check if a key exists in a namespace.
    async fn exists(&self, namespace: &str, key: &str) -> StorageResult<bool>;

    /// List all keys in a namespace.
    async fn list_keys(&self, namespace: &str) -> StorageResult<Vec<String>>;

    /// Delete all keys in a namespace.
    async fn clear_namespace(&self, namespace: &str) -> StorageResult<u64>;
}

// ---------------------------------------------------------------------------
// In-memory implementation (always available)
// ---------------------------------------------------------------------------

/// In-memory key-value store for tests and ephemeral data.
///
/// Keys are stored as `"{namespace}\0{key}"` in a `HashMap`.
#[derive(Debug, Default)]
pub struct MemoryKvStore {
    data: std::sync::RwLock<std::collections::HashMap<String, Vec<u8>>>,
}

impl MemoryKvStore {
    /// Create a new empty in-memory KV store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn full_key(namespace: &str, key: &str) -> String {
        format!("{namespace}\0{key}")
    }
}

#[async_trait]
impl KvStore for MemoryKvStore {
    async fn get(&self, namespace: &str, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let data = self
            .data
            .read()
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        Ok(data.get(&Self::full_key(namespace, key)).cloned())
    }

    async fn set(&self, namespace: &str, key: &str, value: Vec<u8>) -> StorageResult<()> {
        let mut data = self
            .data
            .write()
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        data.insert(Self::full_key(namespace, key), value);
        Ok(())
    }

    async fn delete(&self, namespace: &str, key: &str) -> StorageResult<bool> {
        let mut data = self
            .data
            .write()
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        Ok(data.remove(&Self::full_key(namespace, key)).is_some())
    }

    async fn exists(&self, namespace: &str, key: &str) -> StorageResult<bool> {
        let data = self
            .data
            .read()
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        Ok(data.contains_key(&Self::full_key(namespace, key)))
    }

    async fn list_keys(&self, namespace: &str) -> StorageResult<Vec<String>> {
        let data = self
            .data
            .read()
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        let prefix = format!("{namespace}\0");
        Ok(data
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix).map(String::from))
            .collect())
    }

    async fn clear_namespace(&self, namespace: &str) -> StorageResult<u64> {
        let mut data = self
            .data
            .write()
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        let prefix = format!("{namespace}\0");
        let keys: Vec<String> = data
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        let count = keys.len() as u64;
        for key in keys {
            data.remove(&key);
        }
        Ok(count)
    }
}

// ---------------------------------------------------------------------------
// SurrealKV implementation (behind `kv` feature)
// ---------------------------------------------------------------------------

/// Persistent key-value store backed by `SurrealKV`.
///
/// ACID-compliant, versioned, embedded LSM-tree storage.
/// All operations use transactions internally.
///
/// # Example
///
/// ```rust,ignore
/// use astralis_storage::kv::SurrealKvStore;
///
/// let store = SurrealKvStore::open("./data/kv")?;
/// store.set("wasm:my-plugin", "config", b"{}".to_vec()).await?;
/// ```
#[cfg(feature = "kv")]
pub struct SurrealKvStore {
    tree: surrealkv::Tree,
}

#[cfg(feature = "kv")]
impl std::fmt::Debug for SurrealKvStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurrealKvStore").finish_non_exhaustive()
    }
}

#[cfg(feature = "kv")]
impl SurrealKvStore {
    /// Open a persistent KV store at the given directory path.
    ///
    /// Creates the directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Connection`] if the store cannot be opened.
    pub fn open(path: impl AsRef<std::path::Path>) -> StorageResult<Self> {
        let tree = surrealkv::TreeBuilder::new()
            .with_path(path.as_ref().to_path_buf())
            .build()
            .map_err(|e| StorageError::Connection(e.to_string()))?;
        Ok(Self { tree })
    }

    /// Open a persistent KV store with custom options.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Connection`] if the store cannot be opened.
    pub fn open_with_options(opts: surrealkv::Options) -> StorageResult<Self> {
        let tree = surrealkv::TreeBuilder::with_options(opts)
            .build()
            .map_err(|e| StorageError::Connection(e.to_string()))?;
        Ok(Self { tree })
    }

    /// Close the store, flushing any pending writes.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Internal`] if the flush fails.
    pub async fn close(&self) -> StorageResult<()> {
        self.tree
            .close()
            .await
            .map_err(|e| StorageError::Internal(e.to_string()))
    }
}

#[cfg(feature = "kv")]
fn map_kv_err(e: &surrealkv::Error) -> StorageError {
    StorageError::Internal(e.to_string())
}

#[cfg(feature = "kv")]
#[async_trait]
impl KvStore for SurrealKvStore {
    async fn get(&self, namespace: &str, key: &str) -> StorageResult<Option<Vec<u8>>> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let ck = composite_key(namespace, key);
        let tx = self
            .tree
            .begin_with_mode(surrealkv::Mode::ReadOnly)
            .map_err(|ref e| map_kv_err(e))?;
        tx.get(&ck).map_err(|ref e| map_kv_err(e))
    }

    async fn set(&self, namespace: &str, key: &str, value: Vec<u8>) -> StorageResult<()> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let ck = composite_key(namespace, key);
        let mut tx = self.tree.begin().map_err(|ref e| map_kv_err(e))?;
        tx.set(&ck, &value).map_err(|ref e| map_kv_err(e))?;
        tx.commit().await.map_err(|ref e| map_kv_err(e))
    }

    async fn delete(&self, namespace: &str, key: &str) -> StorageResult<bool> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let ck = composite_key(namespace, key);
        let mut tx = self.tree.begin().map_err(|ref e| map_kv_err(e))?;
        let existed = tx.get(&ck).map_err(|ref e| map_kv_err(e))?.is_some();
        if existed {
            tx.delete(&ck).map_err(|ref e| map_kv_err(e))?;
            tx.commit().await.map_err(|ref e| map_kv_err(e))?;
        }
        Ok(existed)
    }

    async fn exists(&self, namespace: &str, key: &str) -> StorageResult<bool> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let ck = composite_key(namespace, key);
        let tx = self
            .tree
            .begin_with_mode(surrealkv::Mode::ReadOnly)
            .map_err(|ref e| map_kv_err(e))?;
        Ok(tx.get(&ck).map_err(|ref e| map_kv_err(e))?.is_some())
    }

    async fn list_keys(&self, namespace: &str) -> StorageResult<Vec<String>> {
        validate_namespace(namespace)?;
        let start = namespace_range_start(namespace);
        let end = namespace_range_end(namespace);
        let prefix_len = namespace.len() + 1; // namespace + \0

        let tx = self
            .tree
            .begin_with_mode(surrealkv::Mode::ReadOnly)
            .map_err(|ref e| map_kv_err(e))?;
        let mut iter = tx.range(&start, &end).map_err(|ref e| map_kv_err(e))?;
        iter.seek_first().map_err(|ref e| map_kv_err(e))?;

        let mut keys = Vec::new();
        while iter.valid() {
            let raw_key = iter.key();
            if raw_key.len() > prefix_len
                && let Ok(key_str) = std::str::from_utf8(&raw_key[prefix_len..])
            {
                keys.push(key_str.to_string());
            }
            iter.next().map_err(|ref e| map_kv_err(e))?;
        }
        Ok(keys)
    }

    async fn clear_namespace(&self, namespace: &str) -> StorageResult<u64> {
        validate_namespace(namespace)?;
        let start = namespace_range_start(namespace);
        let end = namespace_range_end(namespace);

        let mut tx = self.tree.begin().map_err(|ref e| map_kv_err(e))?;

        // Collect keys first, then delete (iterator borrows tx immutably).
        let keys_to_delete = {
            let mut iter = tx.range(&start, &end).map_err(|ref e| map_kv_err(e))?;
            iter.seek_first().map_err(|ref e| map_kv_err(e))?;
            let mut keys = Vec::new();
            while iter.valid() {
                keys.push(iter.key());
                iter.next().map_err(|ref e| map_kv_err(e))?;
            }
            keys
        }; // iterator dropped — releases immutable borrow on tx

        let count = keys_to_delete.len() as u64;
        for key in &keys_to_delete {
            tx.delete(key).map_err(|ref e| map_kv_err(e))?;
        }
        if count > 0 {
            tx.commit().await.map_err(|ref e| map_kv_err(e))?;
        }
        Ok(count)
    }
}

// ---------------------------------------------------------------------------
// Scoped store (namespace pre-bound)
// ---------------------------------------------------------------------------

/// A namespace-scoped view into a [`KvStore`].
///
/// This is the primary API for WASM guests. The host creates a `ScopedKvStore`
/// per plugin with `namespace = "wasm:{plugin_id}"`, giving the guest simple
/// `get` / `set` / `delete` without ever seeing namespaces.
///
/// Also provides typed convenience via [`get_json`](Self::get_json) /
/// [`set_json`](Self::set_json).
///
/// # Example
///
/// ```rust,ignore
/// use astralis_storage::kv::{ScopedKvStore, MemoryKvStore};
/// use std::sync::Arc;
///
/// let store = Arc::new(MemoryKvStore::new());
/// let scoped = ScopedKvStore::new(store, "wasm:my-plugin")?;
///
/// scoped.set("config", b"{}".to_vec()).await?;
/// let val = scoped.get("config").await?;
/// ```
#[derive(Clone)]
pub struct ScopedKvStore {
    inner: Arc<dyn KvStore>,
    namespace: String,
}

impl std::fmt::Debug for ScopedKvStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedKvStore")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl ScopedKvStore {
    /// Create a scoped view into the given store for `namespace`.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidKey`] if the namespace is empty
    /// or contains null bytes.
    pub fn new(store: Arc<dyn KvStore>, namespace: impl Into<String>) -> StorageResult<Self> {
        let namespace = namespace.into();
        validate_namespace(&namespace)?;
        Ok(Self {
            inner: store,
            namespace,
        })
    }

    /// The namespace this store is scoped to.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Get a raw byte value by key.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidKey`] if the key is empty or invalid.
    pub async fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        validate_key(key)?;
        self.inner.get(&self.namespace, key).await
    }

    /// Set a raw byte value.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidKey`] if the key is empty or invalid.
    pub async fn set(&self, key: &str, value: Vec<u8>) -> StorageResult<()> {
        validate_key(key)?;
        self.inner.set(&self.namespace, key, value).await
    }

    /// Delete a key.
    ///
    /// Returns `true` if the key existed.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidKey`] if the key is empty or invalid.
    pub async fn delete(&self, key: &str) -> StorageResult<bool> {
        validate_key(key)?;
        self.inner.delete(&self.namespace, key).await
    }

    /// Check if a key exists.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidKey`] if the key is empty or invalid.
    pub async fn exists(&self, key: &str) -> StorageResult<bool> {
        validate_key(key)?;
        self.inner.exists(&self.namespace, key).await
    }

    /// List all keys in this namespace.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store operation fails.
    pub async fn list_keys(&self) -> StorageResult<Vec<String>> {
        self.inner.list_keys(&self.namespace).await
    }

    /// Delete all keys in this namespace.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store operation fails.
    pub async fn clear(&self) -> StorageResult<u64> {
        self.inner.clear_namespace(&self.namespace).await
    }

    // -- Typed convenience (JSON) --

    /// Deserialize a JSON value from the store.
    ///
    /// Returns `None` if the key does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Serialization`] if deserialization fails.
    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> StorageResult<Option<T>> {
        let bytes = self.get(key).await?;
        bytes
            .map(|b| {
                serde_json::from_slice(&b).map_err(|e| StorageError::Serialization(e.to_string()))
            })
            .transpose()
    }

    /// Serialize a value as JSON and store it.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Serialization`] if serialization fails.
    pub async fn set_json<T: serde::Serialize>(&self, key: &str, value: &T) -> StorageResult<()> {
        let bytes =
            serde_json::to_vec(value).map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.set(key, bytes).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- MemoryKvStore tests --

    #[tokio::test]
    async fn test_memory_get_set() {
        let store = MemoryKvStore::new();
        store.set("ns1", "key1", b"hello".to_vec()).await.unwrap();
        let val = store.get("ns1", "key1").await.unwrap();
        assert_eq!(val, Some(b"hello".to_vec()));
    }

    #[tokio::test]
    async fn test_memory_get_missing() {
        let store = MemoryKvStore::new();
        let val = store.get("ns1", "missing").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn test_memory_overwrite() {
        let store = MemoryKvStore::new();
        store.set("ns1", "k", b"v1".to_vec()).await.unwrap();
        store.set("ns1", "k", b"v2".to_vec()).await.unwrap();
        let val = store.get("ns1", "k").await.unwrap();
        assert_eq!(val, Some(b"v2".to_vec()));
    }

    #[tokio::test]
    async fn test_memory_delete() {
        let store = MemoryKvStore::new();
        store.set("ns1", "k", b"v".to_vec()).await.unwrap();
        assert!(store.delete("ns1", "k").await.unwrap());
        assert!(!store.delete("ns1", "k").await.unwrap());
        assert!(store.get("ns1", "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_memory_exists() {
        let store = MemoryKvStore::new();
        assert!(!store.exists("ns1", "k").await.unwrap());
        store.set("ns1", "k", b"v".to_vec()).await.unwrap();
        assert!(store.exists("ns1", "k").await.unwrap());
    }

    #[tokio::test]
    async fn test_memory_namespace_isolation() {
        let store = MemoryKvStore::new();
        store.set("ns1", "k", b"v1".to_vec()).await.unwrap();
        store.set("ns2", "k", b"v2".to_vec()).await.unwrap();
        assert_eq!(store.get("ns1", "k").await.unwrap(), Some(b"v1".to_vec()));
        assert_eq!(store.get("ns2", "k").await.unwrap(), Some(b"v2".to_vec()));
    }

    #[tokio::test]
    async fn test_memory_list_keys() {
        let store = MemoryKvStore::new();
        store.set("ns1", "a", b"1".to_vec()).await.unwrap();
        store.set("ns1", "b", b"2".to_vec()).await.unwrap();
        store.set("ns2", "c", b"3".to_vec()).await.unwrap();
        let mut keys = store.list_keys("ns1").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn test_memory_clear_namespace() {
        let store = MemoryKvStore::new();
        store.set("ns1", "a", b"1".to_vec()).await.unwrap();
        store.set("ns1", "b", b"2".to_vec()).await.unwrap();
        store.set("ns2", "c", b"3".to_vec()).await.unwrap();
        let cleared = store.clear_namespace("ns1").await.unwrap();
        assert_eq!(cleared, 2);
        assert!(store.list_keys("ns1").await.unwrap().is_empty());
        assert_eq!(store.list_keys("ns2").await.unwrap().len(), 1);
    }

    // -- Validation tests --

    #[test]
    fn test_validate_namespace_rejects_empty() {
        assert!(validate_namespace("").is_err());
    }

    #[test]
    fn test_validate_namespace_rejects_null_byte() {
        assert!(validate_namespace("ns\0bad").is_err());
    }

    #[test]
    fn test_validate_key_rejects_empty() {
        assert!(validate_key("").is_err());
    }

    #[test]
    fn test_validate_key_rejects_null_byte() {
        assert!(validate_key("k\0bad").is_err());
    }

    // -- ScopedKvStore tests --

    #[tokio::test]
    async fn test_scoped_get_set() {
        let store = Arc::new(MemoryKvStore::new());
        let scoped = ScopedKvStore::new(store, "wasm:plugin-a").unwrap();

        scoped.set("greeting", b"hello".to_vec()).await.unwrap();
        assert_eq!(
            scoped.get("greeting").await.unwrap(),
            Some(b"hello".to_vec())
        );
    }

    #[tokio::test]
    async fn test_scoped_isolation() {
        let store: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
        let a = ScopedKvStore::new(Arc::clone(&store), "wasm:plugin-a").unwrap();
        let b = ScopedKvStore::new(Arc::clone(&store), "wasm:plugin-b").unwrap();

        a.set("key", b"a-value".to_vec()).await.unwrap();
        b.set("key", b"b-value".to_vec()).await.unwrap();

        assert_eq!(a.get("key").await.unwrap(), Some(b"a-value".to_vec()));
        assert_eq!(b.get("key").await.unwrap(), Some(b"b-value".to_vec()));
    }

    #[tokio::test]
    async fn test_scoped_delete_and_exists() {
        let store = Arc::new(MemoryKvStore::new());
        let scoped = ScopedKvStore::new(store, "ns").unwrap();

        assert!(!scoped.exists("k").await.unwrap());
        scoped.set("k", b"v".to_vec()).await.unwrap();
        assert!(scoped.exists("k").await.unwrap());
        assert!(scoped.delete("k").await.unwrap());
        assert!(!scoped.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn test_scoped_list_and_clear() {
        let store = Arc::new(MemoryKvStore::new());
        let scoped = ScopedKvStore::new(store, "ns").unwrap();

        scoped.set("a", b"1".to_vec()).await.unwrap();
        scoped.set("b", b"2".to_vec()).await.unwrap();

        let mut keys = scoped.list_keys().await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);

        assert_eq!(scoped.clear().await.unwrap(), 2);
        assert!(scoped.list_keys().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_scoped_json_round_trip() {
        let store = Arc::new(MemoryKvStore::new());
        let scoped = ScopedKvStore::new(store, "ns").unwrap();

        #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
        struct Config {
            name: String,
            retries: u32,
        }

        let cfg = Config {
            name: "my-plugin".into(),
            retries: 3,
        };
        scoped.set_json("config", &cfg).await.unwrap();

        let loaded: Config = scoped.get_json("config").await.unwrap().unwrap();
        assert_eq!(loaded, cfg);
    }

    #[tokio::test]
    async fn test_scoped_json_missing_returns_none() {
        let store = Arc::new(MemoryKvStore::new());
        let scoped = ScopedKvStore::new(store, "ns").unwrap();

        let val: Option<String> = scoped.get_json("missing").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn test_scoped_rejects_empty_key() {
        let store = Arc::new(MemoryKvStore::new());
        let scoped = ScopedKvStore::new(store, "ns").unwrap();
        assert!(scoped.get("").await.is_err());
    }

    #[test]
    fn test_scoped_rejects_empty_namespace() {
        let store = Arc::new(MemoryKvStore::new());
        assert!(ScopedKvStore::new(store, "").is_err());
    }

    // -- SurrealKvStore tests (behind feature gate) --

    #[cfg(feature = "kv")]
    mod surreal_kv_tests {
        use super::*;

        fn make_store() -> (SurrealKvStore, tempfile::TempDir) {
            let dir = tempfile::tempdir().unwrap();
            let store = SurrealKvStore::open(dir.path()).unwrap();
            (store, dir)
        }

        #[tokio::test]
        async fn test_surreal_get_set() {
            let (store, _dir) = make_store();
            store.set("ns1", "key1", b"hello".to_vec()).await.unwrap();
            let val = store.get("ns1", "key1").await.unwrap();
            assert_eq!(val, Some(b"hello".to_vec()));
        }

        #[tokio::test]
        async fn test_surreal_get_missing() {
            let (store, _dir) = make_store();
            let val = store.get("ns1", "missing").await.unwrap();
            assert!(val.is_none());
        }

        #[tokio::test]
        async fn test_surreal_overwrite() {
            let (store, _dir) = make_store();
            store.set("ns1", "k", b"v1".to_vec()).await.unwrap();
            store.set("ns1", "k", b"v2".to_vec()).await.unwrap();
            let val = store.get("ns1", "k").await.unwrap();
            assert_eq!(val, Some(b"v2".to_vec()));
        }

        #[tokio::test]
        async fn test_surreal_delete() {
            let (store, _dir) = make_store();
            store.set("ns1", "k", b"v".to_vec()).await.unwrap();
            assert!(store.delete("ns1", "k").await.unwrap());
            assert!(!store.delete("ns1", "k").await.unwrap());
            assert!(store.get("ns1", "k").await.unwrap().is_none());
        }

        #[tokio::test]
        async fn test_surreal_exists() {
            let (store, _dir) = make_store();
            assert!(!store.exists("ns1", "k").await.unwrap());
            store.set("ns1", "k", b"v".to_vec()).await.unwrap();
            assert!(store.exists("ns1", "k").await.unwrap());
        }

        #[tokio::test]
        async fn test_surreal_namespace_isolation() {
            let (store, _dir) = make_store();
            store.set("ns1", "k", b"v1".to_vec()).await.unwrap();
            store.set("ns2", "k", b"v2".to_vec()).await.unwrap();
            assert_eq!(store.get("ns1", "k").await.unwrap(), Some(b"v1".to_vec()));
            assert_eq!(store.get("ns2", "k").await.unwrap(), Some(b"v2".to_vec()));
        }

        #[tokio::test]
        async fn test_surreal_list_keys() {
            let (store, _dir) = make_store();
            store.set("ns1", "a", b"1".to_vec()).await.unwrap();
            store.set("ns1", "b", b"2".to_vec()).await.unwrap();
            store.set("ns2", "c", b"3".to_vec()).await.unwrap();
            let mut keys = store.list_keys("ns1").await.unwrap();
            keys.sort();
            assert_eq!(keys, vec!["a", "b"]);
        }

        #[tokio::test]
        async fn test_surreal_clear_namespace() {
            let (store, _dir) = make_store();
            store.set("ns1", "a", b"1".to_vec()).await.unwrap();
            store.set("ns1", "b", b"2".to_vec()).await.unwrap();
            store.set("ns2", "c", b"3".to_vec()).await.unwrap();
            let cleared = store.clear_namespace("ns1").await.unwrap();
            assert_eq!(cleared, 2);
            assert!(store.list_keys("ns1").await.unwrap().is_empty());
            assert_eq!(store.list_keys("ns2").await.unwrap().len(), 1);
        }
    }
}
