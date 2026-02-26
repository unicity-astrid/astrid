//! TTL-based pending request store.
//!
//! Both approval and elicitation managers share a pattern: pending requests
//! stored in a `HashMap`, keyed by a string ID, with automatic TTL-based
//! expiry. This module extracts that pattern into a reusable generic store.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// A single pending entry with its creation timestamp.
struct PendingEntry<V> {
    value: V,
    created_at: Instant,
}

/// A thread-safe store for pending requests with automatic TTL expiry.
///
/// Used by platform-specific `ApprovalManager` and `ElicitationManager`
/// implementations to manage pending requests awaiting user interaction.
///
/// Cloning a `PendingStore` creates a new handle to the same underlying
/// data (via `Arc`), so `V` does not need to implement `Clone`.
pub struct PendingStore<V> {
    entries: Arc<RwLock<HashMap<String, PendingEntry<V>>>>,
    ttl: Duration,
}

impl<V> Clone for PendingStore<V> {
    fn clone(&self) -> Self {
        Self {
            entries: Arc::clone(&self.entries),
            ttl: self.ttl,
        }
    }
}

impl<V> PendingStore<V> {
    /// Create a new, empty pending store with the given TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Insert a pending entry, reaping expired entries first.
    pub async fn insert(&self, key: String, value: V) {
        let mut guard = self.entries.write().await;
        let ttl = self.ttl;
        guard.retain(|_, v| v.created_at.elapsed() < ttl);
        guard.insert(
            key,
            PendingEntry {
                value,
                created_at: Instant::now(),
            },
        );
    }

    /// Remove and return a pending entry by key.
    ///
    /// Returns `None` if the key does not exist, was already consumed, or
    /// has expired (even if not yet reaped).
    pub async fn remove(&self, key: &str) -> Option<V> {
        let entry = self.entries.write().await.remove(key)?;
        if entry.created_at.elapsed() >= self.ttl {
            None
        } else {
            Some(entry.value)
        }
    }

    /// Reap all expired entries. Call periodically to bound memory usage.
    pub async fn reap_expired(&self) {
        let ttl = self.ttl;
        self.entries
            .write()
            .await
            .retain(|_, v| v.created_at.elapsed() < ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_and_remove() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_secs(60));
        store.insert("key1".to_string(), "value1".to_string()).await;

        let val = store.remove("key1").await;
        assert_eq!(val, Some("value1".to_string()));

        // Second remove returns None.
        assert!(store.remove("key1").await.is_none());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_none() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_secs(60));
        assert!(store.remove("missing").await.is_none());
    }

    #[tokio::test]
    async fn insert_reaps_expired() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_millis(1));

        store
            .insert("old".to_string(), "old_value".to_string())
            .await;

        // Wait for TTL to expire.
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Insert a new entry — should reap the expired one.
        store
            .insert("new".to_string(), "new_value".to_string())
            .await;

        assert!(store.remove("old").await.is_none());
        assert_eq!(store.remove("new").await, Some("new_value".to_string()));
    }

    #[tokio::test]
    async fn reap_expired_removes_stale_entries() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_millis(1));

        store.insert("stale".to_string(), "value".to_string()).await;

        tokio::time::sleep(Duration::from_millis(10)).await;

        store.reap_expired().await;
        assert!(store.remove("stale").await.is_none());
    }

    #[tokio::test]
    async fn reap_keeps_fresh_entries() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_secs(60));

        store.insert("fresh".to_string(), "value".to_string()).await;
        store.reap_expired().await;

        assert_eq!(store.remove("fresh").await, Some("value".to_string()));
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let store1: PendingStore<String> = PendingStore::new(Duration::from_secs(60));
        let store2 = store1.clone();

        store1
            .insert("shared".to_string(), "value".to_string())
            .await;
        assert_eq!(store2.remove("shared").await, Some("value".to_string()));
    }

    #[tokio::test]
    async fn remove_rejects_expired_entry() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_millis(1));

        store.insert("key".to_string(), "value".to_string()).await;

        // Wait for TTL to expire without reaping.
        tokio::time::sleep(Duration::from_millis(10)).await;

        // remove() should return None for expired entries even if not reaped.
        assert!(store.remove("key").await.is_none());
    }

    #[tokio::test]
    async fn multiple_entries() {
        let store: PendingStore<i32> = PendingStore::new(Duration::from_secs(60));
        store.insert("a".to_string(), 1).await;
        store.insert("b".to_string(), 2).await;
        store.insert("c".to_string(), 3).await;

        assert_eq!(store.remove("b").await, Some(2));
        assert_eq!(store.remove("a").await, Some(1));
        assert_eq!(store.remove("c").await, Some(3));
    }

    #[tokio::test]
    async fn insert_overwrites_existing_key() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_secs(60));
        store.insert("key".to_string(), "first".to_string()).await;
        store.insert("key".to_string(), "second".to_string()).await;

        assert_eq!(store.remove("key").await, Some("second".to_string()));
        assert!(store.remove("key").await.is_none());
    }

    #[tokio::test]
    async fn non_clone_values() {
        // Verify PendingStore works with non-Clone types.
        struct NonClone(String);
        let store: PendingStore<NonClone> = PendingStore::new(Duration::from_secs(60));
        store
            .insert("k".to_string(), NonClone("hello".to_string()))
            .await;

        let val = store.remove("k").await;
        assert!(val.is_some());
        assert_eq!(val.unwrap().0, "hello");
    }

    #[tokio::test]
    async fn concurrent_insert_and_remove() {
        let store: PendingStore<i32> = PendingStore::new(Duration::from_secs(60));

        // Insert 100 entries concurrently.
        let mut handles = Vec::new();
        for i in 0..100 {
            let s = store.clone();
            handles.push(tokio::spawn(async move {
                s.insert(format!("key-{i}"), i).await;
            }));
        }
        futures::future::join_all(handles).await;

        // Remove all — each should succeed exactly once.
        let mut handles = Vec::new();
        for i in 0..100 {
            let s = store.clone();
            handles.push(tokio::spawn(
                async move { s.remove(&format!("key-{i}")).await },
            ));
        }
        let results: Vec<Option<i32>> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.iter().filter(|r| r.is_some()).count(), 100);
    }

    #[tokio::test]
    async fn reap_with_empty_store_is_noop() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_secs(60));
        store.reap_expired().await;
        // Should not panic or error.
    }

    #[tokio::test]
    async fn mixed_fresh_and_expired_entries() {
        let store: PendingStore<String> = PendingStore::new(Duration::from_millis(50));

        store.insert("old".to_string(), "old_val".to_string()).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        store.insert("new".to_string(), "new_val".to_string()).await;

        // "old" was reaped during insert of "new".
        assert!(store.remove("old").await.is_none());
        assert_eq!(store.remove("new").await, Some("new_val".to_string()));
    }

    #[tokio::test]
    async fn zero_ttl_immediately_expires() {
        let store: PendingStore<String> = PendingStore::new(Duration::ZERO);
        store.insert("k".to_string(), "v".to_string()).await;
        // Even immediate removal should return None since TTL is 0.
        // (Instant::now().elapsed() >= Duration::ZERO is always true.)
        assert!(store.remove("k").await.is_none());
    }
}
