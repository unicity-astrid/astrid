//! Generic session mapping: platform key `K` → daemon `SessionId`.
//!
//! `K` is the platform-specific channel/chat identifier. For Telegram this is
//! `ChatId` (an `i64` newtype); for Discord it could be `ChannelId` (a `u64`
//! newtype) or `UserId`.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;

use astrid_core::SessionId;
use tokio::sync::RwLock;

/// Per-channel session state.
pub struct ChannelSession {
    /// Daemon session ID.
    pub session_id: SessionId,
    /// Whether a turn is currently in progress (prevents double-send).
    pub turn_in_progress: bool,
}

/// Result of attempting to start a turn for a channel.
#[derive(Debug)]
pub enum TurnStartResult {
    /// Turn started successfully; contains the session ID.
    Started(SessionId),
    /// A turn is already in progress (or a session is being created).
    TurnBusy,
    /// No session exists for this channel.
    NoSession,
}

/// Interior state guarded by a single `RwLock`.
struct Inner<K: Eq + Hash> {
    sessions: HashMap<K, ChannelSession>,
    /// Keys that are currently creating a session (prevents duplicate
    /// `create_session` calls when concurrent messages race).
    creating: HashSet<K>,
}

/// Maps platform-specific channel keys to daemon sessions.
///
/// Generic over `K` — any `Eq + Hash + Clone + Send + Sync + 'static` type.
/// Telegram uses `ChatId`; Discord uses `ChannelId` or `UserId`.
#[derive(Clone)]
pub struct SessionMap<K: Eq + Hash + Clone + Send + Sync + 'static> {
    inner: Arc<RwLock<Inner<K>>>,
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> Default for SessionMap<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> SessionMap<K> {
    /// Create an empty session map.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                sessions: HashMap::new(),
                creating: HashSet::new(),
            })),
        }
    }

    /// Get the session ID for a key, if one exists.
    pub async fn get_session_id(&self, key: K) -> Option<SessionId> {
        self.inner
            .read()
            .await
            .sessions
            .get(&key)
            .map(|s| s.session_id.clone())
    }

    /// Insert a new session mapping.
    ///
    /// Also clears any in-progress creation lock for this key to keep
    /// internal invariants consistent.
    pub async fn insert(&self, key: K, session_id: SessionId) {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&key);
        guard.sessions.insert(
            key,
            ChannelSession {
                session_id,
                turn_in_progress: false,
            },
        );
    }

    /// Atomically check if a session exists and start a turn.
    ///
    /// Also returns `TurnBusy` if a session is currently being created for
    /// this key (prevents the caller from starting a duplicate creation).
    pub async fn try_start_existing_turn(&self, key: K) -> TurnStartResult {
        let mut guard = self.inner.write().await;
        if guard.creating.contains(&key) {
            return TurnStartResult::TurnBusy;
        }
        match guard.sessions.get_mut(&key) {
            Some(session) if session.turn_in_progress => TurnStartResult::TurnBusy,
            Some(session) => {
                session.turn_in_progress = true;
                TurnStartResult::Started(session.session_id.clone())
            },
            None => TurnStartResult::NoSession,
        }
    }

    /// Atomically claim the right to create a session for this key.
    ///
    /// Returns `true` if the caller should proceed with `create_session`.
    /// Returns `false` if a session already exists or another task is
    /// already creating one.
    pub async fn try_claim_creation(&self, key: K) -> bool {
        let mut guard = self.inner.write().await;
        if guard.sessions.contains_key(&key) || guard.creating.contains(&key) {
            false
        } else {
            guard.creating.insert(key);
            true
        }
    }

    /// Complete session creation: insert the session and clear the creation
    /// lock.
    pub async fn finish_creation(&self, key: K, session_id: SessionId) {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&key);
        guard.sessions.insert(
            key,
            ChannelSession {
                session_id,
                turn_in_progress: false,
            },
        );
    }

    /// Atomically complete session creation and start a turn in one lock
    /// acquisition. Prevents a race where another message starts the turn
    /// between `finish_creation` and `try_start_existing_turn`.
    pub async fn finish_creation_and_start_turn(&self, key: K, session_id: SessionId) -> SessionId {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&key);
        guard.sessions.insert(
            key,
            ChannelSession {
                session_id: session_id.clone(),
                turn_in_progress: true,
            },
        );
        session_id
    }

    /// Cancel session creation (on failure) and clear the creation lock.
    pub async fn cancel_creation(&self, key: K) {
        self.inner.write().await.creating.remove(&key);
    }

    /// Remove a session mapping.
    ///
    /// Also clears any in-progress creation lock for this key so a
    /// concurrent `finish_creation_and_start_turn` doesn't silently
    /// re-insert the session after a reset.
    pub async fn remove(&self, key: K) -> Option<SessionId> {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&key);
        guard.sessions.remove(&key).map(|s| s.session_id)
    }

    /// Atomically check and start a turn for this key.
    ///
    /// Returns `true` if the turn was started (was not already in progress).
    /// Returns `false` if a turn is already in progress or no session exists.
    pub async fn try_start_turn(&self, key: K) -> bool {
        let mut guard = self.inner.write().await;
        if let Some(session) = guard.sessions.get_mut(&key) {
            if session.turn_in_progress {
                false
            } else {
                session.turn_in_progress = true;
                true
            }
        } else {
            false
        }
    }

    /// Check if a turn is currently in progress for this key.
    pub async fn is_turn_in_progress(&self, key: K) -> bool {
        self.inner
            .read()
            .await
            .sessions
            .get(&key)
            .is_some_and(|s| s.turn_in_progress)
    }

    /// Mark a turn as finished (or in-progress) for this key.
    pub async fn set_turn_in_progress(&self, key: K, in_progress: bool) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(&key) {
            session.turn_in_progress = in_progress;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple newtype to simulate a platform key (like `ChatId` or
    /// `ChannelId`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    struct TestKey(i64);

    fn key(id: i64) -> TestKey {
        TestKey(id)
    }

    #[tokio::test]
    async fn empty_map_returns_none() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.get_session_id(key(1)).await.is_none());
    }

    #[tokio::test]
    async fn insert_and_get() {
        let map: SessionMap<TestKey> = SessionMap::new();
        let sid = SessionId::new();
        map.insert(key(42), sid.clone()).await;

        assert_eq!(map.get_session_id(key(42)).await, Some(sid));
        assert!(map.get_session_id(key(99)).await.is_none());
    }

    #[tokio::test]
    async fn remove_returns_session_and_clears() {
        let map: SessionMap<TestKey> = SessionMap::new();
        let sid = SessionId::new();
        map.insert(key(1), sid.clone()).await;

        let removed = map.remove(key(1)).await;
        assert_eq!(removed, Some(sid));
        assert!(map.get_session_id(key(1)).await.is_none());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_none() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.remove(key(1)).await.is_none());
    }

    #[tokio::test]
    async fn turn_in_progress_defaults_to_false() {
        let map: SessionMap<TestKey> = SessionMap::new();
        map.insert(key(1), SessionId::new()).await;
        assert!(!map.is_turn_in_progress(key(1)).await);
    }

    #[tokio::test]
    async fn turn_in_progress_toggle() {
        let map: SessionMap<TestKey> = SessionMap::new();
        map.insert(key(1), SessionId::new()).await;

        map.set_turn_in_progress(key(1), true).await;
        assert!(map.is_turn_in_progress(key(1)).await);

        map.set_turn_in_progress(key(1), false).await;
        assert!(!map.is_turn_in_progress(key(1)).await);
    }

    #[tokio::test]
    async fn try_start_turn_atomic() {
        let map: SessionMap<TestKey> = SessionMap::new();
        map.insert(key(1), SessionId::new()).await;

        // First call succeeds and sets in_progress.
        assert!(map.try_start_turn(key(1)).await);
        assert!(map.is_turn_in_progress(key(1)).await);

        // Second call fails because already in progress.
        assert!(!map.try_start_turn(key(1)).await);

        // After clearing, can start again.
        map.set_turn_in_progress(key(1), false).await;
        assert!(map.try_start_turn(key(1)).await);
    }

    #[tokio::test]
    async fn try_start_turn_no_session_returns_false() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(!map.try_start_turn(key(999)).await);
    }

    #[tokio::test]
    async fn turn_in_progress_for_unknown_key_is_false() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(!map.is_turn_in_progress(key(999)).await);
    }

    #[tokio::test]
    async fn set_turn_on_unknown_key_is_noop() {
        let map: SessionMap<TestKey> = SessionMap::new();
        // Should not panic.
        map.set_turn_in_progress(key(999), true).await;
        assert!(!map.is_turn_in_progress(key(999)).await);
    }

    #[tokio::test]
    async fn multiple_keys_independent() {
        let map: SessionMap<TestKey> = SessionMap::new();
        let sid1 = SessionId::new();
        let sid2 = SessionId::new();
        map.insert(key(1), sid1.clone()).await;
        map.insert(key(2), sid2.clone()).await;

        map.set_turn_in_progress(key(1), true).await;
        assert!(map.is_turn_in_progress(key(1)).await);
        assert!(!map.is_turn_in_progress(key(2)).await);

        assert_eq!(map.get_session_id(key(1)).await, Some(sid1));
        assert_eq!(map.get_session_id(key(2)).await, Some(sid2));
    }

    #[tokio::test]
    async fn insert_overwrites_existing() {
        let map: SessionMap<TestKey> = SessionMap::new();
        let sid1 = SessionId::new();
        let sid2 = SessionId::new();

        map.insert(key(1), sid1).await;
        map.set_turn_in_progress(key(1), true).await;

        // Overwrite with new session.
        map.insert(key(1), sid2.clone()).await;

        assert_eq!(map.get_session_id(key(1)).await, Some(sid2));
        // turn_in_progress should be reset to false.
        assert!(!map.is_turn_in_progress(key(1)).await);
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let map1: SessionMap<TestKey> = SessionMap::new();
        let map2 = map1.clone();
        let sid = SessionId::new();

        map1.insert(key(1), sid.clone()).await;
        assert_eq!(map2.get_session_id(key(1)).await, Some(sid));
    }

    // --- creation lock ---

    #[tokio::test]
    async fn try_claim_creation_succeeds_when_no_session() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);
    }

    #[tokio::test]
    async fn try_claim_creation_fails_when_already_creating() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);
        // Second call for same key should fail.
        assert!(!map.try_claim_creation(key(1)).await);
    }

    #[tokio::test]
    async fn try_claim_creation_fails_when_session_exists() {
        let map: SessionMap<TestKey> = SessionMap::new();
        map.insert(key(1), SessionId::new()).await;
        assert!(!map.try_claim_creation(key(1)).await);
    }

    #[tokio::test]
    async fn finish_creation_inserts_session_and_clears_lock() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);

        let sid = SessionId::new();
        map.finish_creation(key(1), sid.clone()).await;

        assert_eq!(map.get_session_id(key(1)).await, Some(sid));
        assert!(!map.try_claim_creation(key(1)).await);
    }

    #[tokio::test]
    async fn cancel_creation_clears_lock() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);
        map.cancel_creation(key(1)).await;
        // Lock is cleared, can try again.
        assert!(map.try_claim_creation(key(1)).await);
    }

    #[tokio::test]
    async fn creating_blocks_try_start_existing_turn() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);
        // While creating, try_start_existing_turn should return TurnBusy.
        assert!(matches!(
            map.try_start_existing_turn(key(1)).await,
            TurnStartResult::TurnBusy
        ));
    }

    // --- try_start_existing_turn branches ---

    #[tokio::test]
    async fn try_start_existing_turn_no_session() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(matches!(
            map.try_start_existing_turn(key(1)).await,
            TurnStartResult::NoSession
        ));
    }

    #[tokio::test]
    async fn try_start_existing_turn_started() {
        let map: SessionMap<TestKey> = SessionMap::new();
        let sid = SessionId::new();
        map.insert(key(1), sid.clone()).await;

        match map.try_start_existing_turn(key(1)).await {
            TurnStartResult::Started(returned_sid) => {
                assert_eq!(returned_sid, sid);
            },
            other => panic!("Expected Started, got {other:?}"),
        }
        assert!(map.is_turn_in_progress(key(1)).await);
    }

    #[tokio::test]
    async fn try_start_existing_turn_busy() {
        let map: SessionMap<TestKey> = SessionMap::new();
        map.insert(key(1), SessionId::new()).await;

        // Start first turn.
        assert!(matches!(
            map.try_start_existing_turn(key(1)).await,
            TurnStartResult::Started(_)
        ));
        // Second attempt should be busy.
        assert!(matches!(
            map.try_start_existing_turn(key(1)).await,
            TurnStartResult::TurnBusy
        ));
    }

    // --- finish_creation_and_start_turn ---

    #[tokio::test]
    async fn finish_creation_and_start_turn_inserts_and_starts() {
        let map: SessionMap<TestKey> = SessionMap::new();
        let sid = SessionId::new();
        assert!(map.try_claim_creation(key(1)).await);

        let returned = map
            .finish_creation_and_start_turn(key(1), sid.clone())
            .await;
        assert_eq!(returned, sid);
        assert_eq!(map.get_session_id(key(1)).await, Some(sid));
        assert!(map.is_turn_in_progress(key(1)).await);
    }

    #[tokio::test]
    async fn finish_creation_and_start_turn_clears_creation_lock() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);
        map.finish_creation_and_start_turn(key(1), SessionId::new())
            .await;
        // Creation lock cleared — cannot re-claim.
        assert!(!map.try_claim_creation(key(1)).await);
    }

    // --- remove clears creation lock ---

    #[tokio::test]
    async fn remove_clears_creation_lock() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);
        // Insert a session (as if creation completed).
        map.insert(key(1), SessionId::new()).await;

        // Remove should clear the creation lock too.
        map.remove(key(1)).await;
        // Now we should be able to claim creation again.
        assert!(map.try_claim_creation(key(1)).await);
    }

    #[tokio::test]
    async fn remove_during_creation_clears_lock() {
        let map: SessionMap<TestKey> = SessionMap::new();
        assert!(map.try_claim_creation(key(1)).await);

        // Remove without ever inserting — should clear creation lock.
        assert!(map.remove(key(1)).await.is_none());
        assert!(map.try_claim_creation(key(1)).await);
    }

    // --- concurrent access ---

    #[tokio::test]
    async fn concurrent_turn_starts_only_one_wins() {
        let map: SessionMap<TestKey> = SessionMap::new();
        map.insert(key(1), SessionId::new()).await;

        let mut handles = Vec::new();
        for _ in 0..10 {
            let m = map.clone();
            handles.push(tokio::spawn(async move { m.try_start_turn(key(1)).await }));
        }

        let results: Vec<bool> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Exactly one task should have won.
        assert_eq!(results.iter().filter(|&&v| v).count(), 1);
    }

    #[tokio::test]
    async fn concurrent_claim_creation_only_one_wins() {
        let map: SessionMap<TestKey> = SessionMap::new();

        let mut handles = Vec::new();
        for _ in 0..10 {
            let m = map.clone();
            handles.push(tokio::spawn(
                async move { m.try_claim_creation(key(1)).await },
            ));
        }

        let results: Vec<bool> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.iter().filter(|&&v| v).count(), 1);
    }

    // --- default trait ---

    #[tokio::test]
    async fn default_creates_empty_map() {
        let map: SessionMap<TestKey> = SessionMap::default();
        assert!(map.get_session_id(key(1)).await.is_none());
    }

    // --- string keys ---

    #[tokio::test]
    async fn string_keys_work() {
        let map: SessionMap<String> = SessionMap::new();
        let sid = SessionId::new();
        map.insert("channel-123".to_string(), sid.clone()).await;

        assert_eq!(
            map.get_session_id("channel-123".to_string()).await,
            Some(sid)
        );
    }
}
