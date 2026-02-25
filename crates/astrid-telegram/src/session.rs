//! Session mapping: Telegram `ChatId` → daemon `SessionId`.
//!
//! This module re-exports the generic [`astrid_frontend_common::SessionMap`]
//! specialized for Telegram's `ChatId`.

pub use astrid_frontend_common::session::{ChannelSession, TurnStartResult};

use teloxide::types::ChatId;

/// Telegram-specific session map: `ChatId` → daemon session.
pub type SessionMap = astrid_frontend_common::SessionMap<ChatId>;

/// Legacy alias — [`ChannelSession`] is the shared name.
pub type ChatSession = ChannelSession;

#[cfg(test)]
mod tests {
    use super::*;
    use astrid_core::SessionId;

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
