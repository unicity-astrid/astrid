//! KV-based session management for the Discord capsule.
//!
//! Maps Discord channel IDs (or user IDs) to Astrid runtime sessions
//! using the KV Airlock for persistence.

use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// Per-channel (or per-user) session state, persisted in KV.
#[derive(Serialize, Deserialize)]
pub(crate) struct SessionState {
    /// Astrid runtime session ID (from Uplink send result).
    pub session_id: String,
    /// Registered Uplink connector UUID.
    pub connector_id: String,
    /// Last Discord message ID (for editing streamed responses).
    pub last_message_id: Option<String>,
    /// Whether an agent turn is currently in progress.
    pub turn_in_progress: bool,
    /// Interaction token for deferred responses (valid ~15 min).
    pub interaction_token: Option<String>,
}

/// Capsule initialization state, persisted in KV.
///
/// Credentials (bot token, app ID) are NOT stored here — they are read
/// from Sys config on each use via `DiscordApi::from_config()`.
#[derive(Serialize, Deserialize)]
pub(crate) struct InitState {
    /// Registered Uplink connector UUID.
    pub connector_id: String,
    /// IPC subscription handle for agent events.
    pub event_handle: String,
}

/// Metadata for the currently active agent turn.
///
/// Stored in KV so that event handlers (`text_chunk`, `turn_complete`,
/// `error`) can resolve the correct session scope without relying on
/// `channel_id` from the event payload.
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct ActiveTurn {
    /// Session scope mode ("channel" or "user").
    pub scope: String,
    /// The resolved scope ID (channel_id or user_id).
    pub scope_id: String,
    /// Accumulated text buffer for streaming edits.
    pub buffer: String,
    /// The Discord channel ID where responses should be sent.
    /// Always set for Gateway message turns; may be empty for
    /// interaction turns (which use Discord's interaction webhook
    /// API tokens via `interaction_token` instead).
    #[serde(default)]
    pub channel_id: String,
}

/// Load the active turn metadata from KV.
pub(crate) fn get_active_turn() -> Result<Option<ActiveTurn>, SysError> {
    match kv::get_json::<ActiveTurn>("turn:active") {
        Ok(turn) => Ok(Some(turn)),
        Err(_) => Ok(None),
    }
}

/// Save the active turn metadata to KV.
pub(crate) fn set_active_turn(turn: &ActiveTurn) -> Result<(), SysError> {
    kv::set_json("turn:active", turn)
}

/// Clear the active turn metadata.
pub(crate) fn clear_active_turn() -> Result<(), SysError> {
    kv::set_bytes("turn:active", &[])
}

/// Build the KV key for a session.
///
/// Uses `session:channel:{id}` or `session:user:{id}` depending on
/// the configured scope.
pub(crate) fn session_key(scope: SessionScope, id: &str) -> String {
    match scope {
        SessionScope::Channel => format!("session:channel:{id}"),
        SessionScope::User => format!("session:user:{id}"),
    }
}

/// Session scoping mode.
#[derive(Clone, Copy)]
pub(crate) enum SessionScope {
    /// One session per Discord channel.
    Channel,
    /// One session per Discord user.
    User,
}

impl SessionScope {
    /// Parse from a configuration string.
    pub(crate) fn from_config(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "user" => Self::User,
            _ => Self::Channel,
        }
    }
}

/// Load a session from KV.
pub(crate) fn get_session(scope: SessionScope, id: &str) -> Result<Option<SessionState>, SysError> {
    let key = session_key(scope, id);
    match kv::get_json::<SessionState>(&key) {
        Ok(state) => Ok(Some(state)),
        Err(_) => Ok(None),
    }
}

/// Save a session to KV.
pub(crate) fn set_session(
    scope: SessionScope,
    id: &str,
    state: &SessionState,
) -> Result<(), SysError> {
    let key = session_key(scope, id);
    kv::set_json(&key, state)
}

/// Remove a session from KV.
pub(crate) fn remove_session(scope: SessionScope, id: &str) -> Result<(), SysError> {
    let key = session_key(scope, id);
    // Write empty bytes to effectively clear the key.
    kv::set_bytes(&key, &[])
}

/// Load the capsule initialization state.
pub(crate) fn get_init_state() -> Result<Option<InitState>, SysError> {
    match kv::get_json::<InitState>("init:state") {
        Ok(state) => Ok(Some(state)),
        Err(_) => Ok(None),
    }
}

/// Save the capsule initialization state.
pub(crate) fn set_init_state(state: &InitState) -> Result<(), SysError> {
    kv::set_json("init:state", state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_channel() {
        let key = session_key(SessionScope::Channel, "12345");
        assert_eq!(key, "session:channel:12345");
    }

    #[test]
    fn session_key_user() {
        let key = session_key(SessionScope::User, "99999");
        assert_eq!(key, "session:user:99999");
    }

    #[test]
    fn scope_from_config_default() {
        assert!(matches!(
            SessionScope::from_config("channel"),
            SessionScope::Channel
        ));
        assert!(matches!(
            SessionScope::from_config(""),
            SessionScope::Channel
        ));
        assert!(matches!(
            SessionScope::from_config("unknown"),
            SessionScope::Channel
        ));
    }

    #[test]
    fn scope_from_config_user() {
        assert!(matches!(
            SessionScope::from_config("user"),
            SessionScope::User
        ));
        assert!(matches!(
            SessionScope::from_config("  User  "),
            SessionScope::User
        ));
    }

    #[test]
    fn scope_from_config_case_insensitive() {
        assert!(matches!(
            SessionScope::from_config("USER"),
            SessionScope::User
        ));
        assert!(matches!(
            SessionScope::from_config("CHANNEL"),
            SessionScope::Channel
        ));
        assert!(matches!(
            SessionScope::from_config("Channel"),
            SessionScope::Channel
        ));
    }

    #[test]
    fn session_key_empty_id() {
        assert_eq!(session_key(SessionScope::Channel, ""), "session:channel:");
        assert_eq!(session_key(SessionScope::User, ""), "session:user:");
    }

    #[test]
    fn session_key_special_chars() {
        let key = session_key(SessionScope::Channel, "guild:123/channel:456");
        assert_eq!(key, "session:channel:guild:123/channel:456");
    }

    // --- SessionState serde round-trip ---

    #[test]
    fn session_state_serde_round_trip() {
        let state = SessionState {
            session_id: "sess-42".to_string(),
            connector_id: "conn-1".to_string(),
            last_message_id: Some("msg-99".to_string()),
            turn_in_progress: true,
            interaction_token: Some("token-abc".to_string()),
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.session_id, "sess-42");
        assert_eq!(restored.connector_id, "conn-1");
        assert_eq!(restored.last_message_id.as_deref(), Some("msg-99"));
        assert!(restored.turn_in_progress);
        assert_eq!(restored.interaction_token.as_deref(), Some("token-abc"));
    }

    #[test]
    fn session_state_serde_none_fields() {
        let state = SessionState {
            session_id: "s".to_string(),
            connector_id: "c".to_string(),
            last_message_id: None,
            turn_in_progress: false,
            interaction_token: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();

        assert!(restored.last_message_id.is_none());
        assert!(!restored.turn_in_progress);
        assert!(restored.interaction_token.is_none());
    }

    // --- InitState serde round-trip ---

    #[test]
    fn init_state_serde_round_trip() {
        let state = InitState {
            connector_id: "uuid-1234".to_string(),
            event_handle: "handle-5678".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: InitState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.connector_id, "uuid-1234");
        assert_eq!(restored.event_handle, "handle-5678");
    }

    // --- ActiveTurn serde round-trip ---

    #[test]
    fn active_turn_serde_round_trip() {
        let turn = ActiveTurn {
            scope: "channel".to_string(),
            scope_id: "ch-123".to_string(),
            buffer: "accumulated text".to_string(),
            channel_id: "ch-123".to_string(),
        };
        let json = serde_json::to_string(&turn).unwrap();
        let restored: ActiveTurn = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.scope, "channel");
        assert_eq!(restored.scope_id, "ch-123");
        assert_eq!(restored.buffer, "accumulated text");
        assert_eq!(restored.channel_id, "ch-123");
    }

    #[test]
    fn active_turn_default() {
        let turn = ActiveTurn::default();
        assert!(turn.scope.is_empty());
        assert!(turn.scope_id.is_empty());
        assert!(turn.buffer.is_empty());
        assert!(turn.channel_id.is_empty());
    }

    #[test]
    fn active_turn_backward_compat() {
        // Old serialized data without channel_id should still deserialize.
        let json = r#"{"scope":"channel","scope_id":"ch-1","buffer":""}"#;
        let turn: ActiveTurn = serde_json::from_str(json).unwrap();
        assert!(turn.channel_id.is_empty());
    }
}
