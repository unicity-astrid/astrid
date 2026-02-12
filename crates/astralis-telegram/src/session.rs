//! Session mapping: Telegram `ChatId` → daemon `SessionId`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use astralis_core::SessionId;
use teloxide::types::ChatId;
use tokio::sync::RwLock;

/// Per-chat session state.
pub struct ChatSession {
    /// Daemon session ID.
    pub session_id: SessionId,
    /// Whether a turn is currently in progress (prevents double-send).
    pub turn_in_progress: bool,
}

/// Result of attempting to start a turn for a chat.
pub enum TurnStartResult {
    /// Turn started successfully; contains the session ID.
    Started(SessionId),
    /// A turn is already in progress (or a session is being created).
    TurnBusy,
    /// No session exists for this chat.
    NoSession,
}

/// Interior state guarded by a single `RwLock`.
struct Inner {
    sessions: HashMap<ChatId, ChatSession>,
    /// Chats that are currently creating a session (prevents duplicate
    /// `create_session` calls when concurrent messages race).
    creating: HashSet<ChatId>,
}

/// Maps Telegram chat IDs to daemon sessions.
#[derive(Clone)]
pub struct SessionMap {
    inner: Arc<RwLock<Inner>>,
}

impl Default for SessionMap {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionMap {
    /// Create an empty session map.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                sessions: HashMap::new(),
                creating: HashSet::new(),
            })),
        }
    }

    /// Get the session ID for a chat, if one exists.
    pub async fn get_session_id(&self, chat_id: ChatId) -> Option<SessionId> {
        self.inner
            .read()
            .await
            .sessions
            .get(&chat_id)
            .map(|s| s.session_id.clone())
    }

    /// Insert a new session mapping.
    ///
    /// Also clears any in-progress creation lock for this chat to keep
    /// internal invariants consistent.
    pub async fn insert(&self, chat_id: ChatId, session_id: SessionId) {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&chat_id);
        guard.sessions.insert(
            chat_id,
            ChatSession {
                session_id,
                turn_in_progress: false,
            },
        );
    }

    /// Atomically check if a session exists and start a turn.
    ///
    /// Also returns `TurnBusy` if a session is currently being created for
    /// this chat (prevents the caller from starting a duplicate creation).
    pub async fn try_start_existing_turn(&self, chat_id: ChatId) -> TurnStartResult {
        let mut guard = self.inner.write().await;
        if guard.creating.contains(&chat_id) {
            return TurnStartResult::TurnBusy;
        }
        match guard.sessions.get_mut(&chat_id) {
            Some(session) if session.turn_in_progress => TurnStartResult::TurnBusy,
            Some(session) => {
                session.turn_in_progress = true;
                TurnStartResult::Started(session.session_id.clone())
            },
            None => TurnStartResult::NoSession,
        }
    }

    /// Atomically claim the right to create a session for this chat.
    ///
    /// Returns `true` if the caller should proceed with `create_session`.
    /// Returns `false` if a session already exists or another task is
    /// already creating one.
    pub async fn try_claim_creation(&self, chat_id: ChatId) -> bool {
        let mut guard = self.inner.write().await;
        if guard.sessions.contains_key(&chat_id) || guard.creating.contains(&chat_id) {
            false
        } else {
            guard.creating.insert(chat_id);
            true
        }
    }

    /// Complete session creation: insert the session and clear the creation
    /// lock.
    pub async fn finish_creation(&self, chat_id: ChatId, session_id: SessionId) {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&chat_id);
        guard.sessions.insert(
            chat_id,
            ChatSession {
                session_id,
                turn_in_progress: false,
            },
        );
    }

    /// Atomically complete session creation and start a turn in one lock
    /// acquisition. Prevents a race where another message starts the turn
    /// between `finish_creation` and `try_start_existing_turn`.
    pub async fn finish_creation_and_start_turn(
        &self,
        chat_id: ChatId,
        session_id: SessionId,
    ) -> SessionId {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&chat_id);
        guard.sessions.insert(
            chat_id,
            ChatSession {
                session_id: session_id.clone(),
                turn_in_progress: true,
            },
        );
        session_id
    }

    /// Cancel session creation (on failure) and clear the creation lock.
    pub async fn cancel_creation(&self, chat_id: ChatId) {
        self.inner.write().await.creating.remove(&chat_id);
    }

    /// Remove a session mapping.
    ///
    /// Also clears any in-progress creation lock for this chat so a
    /// concurrent `finish_creation_and_start_turn` doesn't silently
    /// re-insert the session after a `/reset`.
    pub async fn remove(&self, chat_id: ChatId) -> Option<SessionId> {
        let mut guard = self.inner.write().await;
        guard.creating.remove(&chat_id);
        guard.sessions.remove(&chat_id).map(|s| s.session_id)
    }

    /// Atomically check and start a turn for this chat.
    ///
    /// Returns `true` if the turn was started (was not already in progress).
    /// Returns `false` if a turn is already in progress or no session exists.
    pub async fn try_start_turn(&self, chat_id: ChatId) -> bool {
        let mut guard = self.inner.write().await;
        if let Some(session) = guard.sessions.get_mut(&chat_id) {
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

    /// Check if a turn is currently in progress for this chat.
    pub async fn is_turn_in_progress(&self, chat_id: ChatId) -> bool {
        self.inner
            .read()
            .await
            .sessions
            .get(&chat_id)
            .is_some_and(|s| s.turn_in_progress)
    }

    /// Mark a turn as finished for this chat.
    pub async fn set_turn_in_progress(&self, chat_id: ChatId, in_progress: bool) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(&chat_id) {
            session.turn_in_progress = in_progress;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: i64) -> ChatId {
        ChatId(id)
    }

    #[tokio::test]
    async fn empty_map_returns_none() {
        let map = SessionMap::new();
        assert!(map.get_session_id(chat(1)).await.is_none());
    }

    #[tokio::test]
    async fn insert_and_get() {
        let map = SessionMap::new();
        let sid = SessionId::new();
        map.insert(chat(42), sid.clone()).await;

        assert_eq!(map.get_session_id(chat(42)).await, Some(sid));
        assert!(map.get_session_id(chat(99)).await.is_none());
    }

    #[tokio::test]
    async fn remove_returns_session_and_clears() {
        let map = SessionMap::new();
        let sid = SessionId::new();
        map.insert(chat(1), sid.clone()).await;

        let removed = map.remove(chat(1)).await;
        assert_eq!(removed, Some(sid));
        assert!(map.get_session_id(chat(1)).await.is_none());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_none() {
        let map = SessionMap::new();
        assert!(map.remove(chat(1)).await.is_none());
    }

    #[tokio::test]
    async fn turn_in_progress_defaults_to_false() {
        let map = SessionMap::new();
        map.insert(chat(1), SessionId::new()).await;
        assert!(!map.is_turn_in_progress(chat(1)).await);
    }

    #[tokio::test]
    async fn turn_in_progress_toggle() {
        let map = SessionMap::new();
        map.insert(chat(1), SessionId::new()).await;

        map.set_turn_in_progress(chat(1), true).await;
        assert!(map.is_turn_in_progress(chat(1)).await);

        map.set_turn_in_progress(chat(1), false).await;
        assert!(!map.is_turn_in_progress(chat(1)).await);
    }

    #[tokio::test]
    async fn try_start_turn_atomic() {
        let map = SessionMap::new();
        map.insert(chat(1), SessionId::new()).await;

        // First call succeeds and sets in_progress.
        assert!(map.try_start_turn(chat(1)).await);
        assert!(map.is_turn_in_progress(chat(1)).await);

        // Second call fails because already in progress.
        assert!(!map.try_start_turn(chat(1)).await);

        // After clearing, can start again.
        map.set_turn_in_progress(chat(1), false).await;
        assert!(map.try_start_turn(chat(1)).await);
    }

    #[tokio::test]
    async fn try_start_turn_no_session_returns_false() {
        let map = SessionMap::new();
        assert!(!map.try_start_turn(chat(999)).await);
    }

    #[tokio::test]
    async fn turn_in_progress_for_unknown_chat_is_false() {
        let map = SessionMap::new();
        assert!(!map.is_turn_in_progress(chat(999)).await);
    }

    #[tokio::test]
    async fn set_turn_on_unknown_chat_is_noop() {
        let map = SessionMap::new();
        // Should not panic.
        map.set_turn_in_progress(chat(999), true).await;
        assert!(!map.is_turn_in_progress(chat(999)).await);
    }

    #[tokio::test]
    async fn multiple_chats_independent() {
        let map = SessionMap::new();
        let sid1 = SessionId::new();
        let sid2 = SessionId::new();
        map.insert(chat(1), sid1.clone()).await;
        map.insert(chat(2), sid2.clone()).await;

        map.set_turn_in_progress(chat(1), true).await;
        assert!(map.is_turn_in_progress(chat(1)).await);
        assert!(!map.is_turn_in_progress(chat(2)).await);

        assert_eq!(map.get_session_id(chat(1)).await, Some(sid1));
        assert_eq!(map.get_session_id(chat(2)).await, Some(sid2));
    }

    #[tokio::test]
    async fn insert_overwrites_existing() {
        let map = SessionMap::new();
        let sid1 = SessionId::new();
        let sid2 = SessionId::new();

        map.insert(chat(1), sid1).await;
        map.set_turn_in_progress(chat(1), true).await;

        // Overwrite with new session.
        map.insert(chat(1), sid2.clone()).await;

        assert_eq!(map.get_session_id(chat(1)).await, Some(sid2));
        // turn_in_progress should be reset to false.
        assert!(!map.is_turn_in_progress(chat(1)).await);
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let map1 = SessionMap::new();
        let map2 = map1.clone();
        let sid = SessionId::new();

        map1.insert(chat(1), sid.clone()).await;
        assert_eq!(map2.get_session_id(chat(1)).await, Some(sid));
    }

    // --- creation lock ---

    #[tokio::test]
    async fn try_claim_creation_succeeds_when_no_session() {
        let map = SessionMap::new();
        assert!(map.try_claim_creation(chat(1)).await);
    }

    #[tokio::test]
    async fn try_claim_creation_fails_when_already_creating() {
        let map = SessionMap::new();
        assert!(map.try_claim_creation(chat(1)).await);
        // Second call for same chat should fail.
        assert!(!map.try_claim_creation(chat(1)).await);
    }

    #[tokio::test]
    async fn try_claim_creation_fails_when_session_exists() {
        let map = SessionMap::new();
        map.insert(chat(1), SessionId::new()).await;
        assert!(!map.try_claim_creation(chat(1)).await);
    }

    #[tokio::test]
    async fn finish_creation_inserts_session_and_clears_lock() {
        let map = SessionMap::new();
        assert!(map.try_claim_creation(chat(1)).await);

        let sid = SessionId::new();
        map.finish_creation(chat(1), sid.clone()).await;

        assert_eq!(map.get_session_id(chat(1)).await, Some(sid));
        // Creation lock should be cleared — can claim again if needed.
        // (In practice, session exists so claim would fail for a different reason.)
        assert!(!map.try_claim_creation(chat(1)).await);
    }

    #[tokio::test]
    async fn cancel_creation_clears_lock() {
        let map = SessionMap::new();
        assert!(map.try_claim_creation(chat(1)).await);
        map.cancel_creation(chat(1)).await;
        // Lock is cleared, can try again.
        assert!(map.try_claim_creation(chat(1)).await);
    }

    #[tokio::test]
    async fn creating_blocks_try_start_existing_turn() {
        let map = SessionMap::new();
        assert!(map.try_claim_creation(chat(1)).await);
        // While creating, try_start_existing_turn should return TurnBusy.
        assert!(matches!(
            map.try_start_existing_turn(chat(1)).await,
            TurnStartResult::TurnBusy
        ));
    }
}
