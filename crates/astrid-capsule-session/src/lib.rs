#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![warn(missing_docs)]

//! Session capsule for Astrid OS.
//!
//! Dumb, trustworthy store for conversation history. Holds clean messages:
//! what the user said, what the assistant replied, what tools returned.
//! Never transforms anything. Clean in, clean out.
//!
//! The react loop (or any future replacement) appends messages at turn
//! boundaries and fetches history when building LLM requests. Prompt
//! builder injections, system prompt assembly, context compaction -
//! those are ephemeral per-turn transforms that never touch session.
//!
//! # Session chaining
//!
//! Sessions form a linked list via `parent_session_id`. When a session
//! is cleared or compacted, a new session is created pointing back to
//! the old one. History is never silently truncated.

use astrid_events::llm::Message;
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// KV key prefix for session data.
const SESSION_KEY_PREFIX: &str = "session.data";

/// Default session ID.
const DEFAULT_SESSION_ID: &str = "default";

/// Current schema version for `SessionData`.
const SESSION_DATA_SCHEMA_VERSION: u32 = 1;

/// Build the KV key for a session's data.
fn session_key(session_id: &str) -> String {
    format!("{SESSION_KEY_PREFIX}.{session_id}")
}

/// Persistent conversation session data.
///
/// Schema-versioned for forward-compatible deserialization. On load:
/// - v0 (legacy, missing field): stamp to v1 and re-save.
/// - v1 (current): use as-is.
/// - Unknown future version: log error, start fresh (fail secure).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionData {
    /// Schema version. Defaults to 0 for pre-versioning data.
    #[serde(default)]
    schema_version: u32,
    /// Previous session in the chain, if this session was created
    /// via clear or compaction.
    #[serde(default)]
    parent_session_id: Option<String>,
    /// Clean conversation message history.
    messages: Vec<Message>,
}

impl Default for SessionData {
    fn default() -> Self {
        Self {
            schema_version: SESSION_DATA_SCHEMA_VERSION,
            parent_session_id: None,
            messages: Vec::new(),
        }
    }
}

impl SessionData {
    /// Apply schema migration to deserialized data (pure logic, no I/O).
    ///
    /// Returns `Ok((data, needs_save))` if migration succeeded or was
    /// unnecessary. `needs_save` is true when the version was bumped.
    /// Returns `Err(fresh_default)` if the version is unrecognized
    /// (fail secure).
    fn migrate(mut self) -> Result<(Self, bool), Self> {
        match self.schema_version {
            0 => {
                self.schema_version = SESSION_DATA_SCHEMA_VERSION;
                Ok((self, true))
            },
            v if v == SESSION_DATA_SCHEMA_VERSION => Ok((self, false)),
            _ => Err(Self::default()),
        }
    }

    /// Load session data from KV, applying schema migration as needed.
    fn load(session_id: &str) -> Self {
        let key = session_key(session_id);
        let data = kv::get_json::<Self>(&key).unwrap_or_else(|e| {
            let _ = sys::log(
                "error",
                format!("Failed to load session data, starting fresh: {e}"),
            );
            Self::default()
        });

        match data.migrate() {
            Ok((migrated, needs_save)) => {
                // If version was bumped (v0 -> v1), persist the migration.
                // No retry on save failure - the in-memory data is still
                // usable and re-save will be attempted on next modification.
                if needs_save {
                    if let Err(e) = migrated.save(session_id) {
                        let _ = sys::log(
                            "warn",
                            format!("Failed to re-save session after migration: {e}"),
                        );
                    }
                }
                migrated
            },
            Err(fresh) => {
                let _ = sys::log(
                    "error",
                    format!(
                        "Session '{session_id}' has unknown schema version \
                         (expected {SESSION_DATA_SCHEMA_VERSION}), starting fresh"
                    ),
                );
                fresh
            },
        }
    }

    /// Persist session data to KV.
    fn save(&self, session_id: &str) -> Result<(), SysError> {
        let key = session_key(session_id);
        kv::set_json(&key, self)
    }
}

/// Session capsule. Dumb store with session chaining.
///
/// # Security note
///
/// Session isolation (restricting which capsules can read/write which
/// session IDs) is enforced at the kernel's topic ACL layer, not within
/// this capsule. Any capsule with `ipc_publish` permission for the
/// `session.request.*` topics can access any session by ID.
#[derive(Default)]
pub struct Session;

#[capsule]
impl Session {
    /// Handles `session.append` events.
    ///
    /// Appends one or more messages to the conversation history.
    /// Fire-and-forget - no response published.
    ///
    /// The react capsule uses `append_before_read` on `get_messages` for
    /// atomic appends. This standalone handler exists as a public API for
    /// other capsules that need to inject messages without reading history
    /// (e.g. system notifications, external integrations).
    #[astrid::interceptor("handle_append")]
    pub fn handle_append(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_SESSION_ID);

        let messages: Vec<Message> = payload
            .get("messages")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| SysError::ApiError(format!("Failed to parse messages: {e}")))?
            .unwrap_or_default();

        if messages.is_empty() {
            return Ok(());
        }

        let mut data = SessionData::load(session_id);
        data.messages.extend(messages);
        data.save(session_id)
    }

    /// Handles `session.request.get_messages` events.
    ///
    /// Returns the conversation history to the requester via a per-request
    /// scoped reply topic (`session.v1.response.get_messages.<correlation_id>`).
    /// This prevents cross-instance response theft under concurrent load.
    ///
    /// Supports an optional `append_before_read` field containing messages
    /// to append atomically before returning the history. This eliminates
    /// the race between a separate `session.append` and `get_messages`.
    #[astrid::interceptor("handle_get_messages")]
    pub fn handle_get_messages(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_SESSION_ID);

        // correlation_id is interpolated into the reply topic as a single
        // segment. Reject empty values and values containing dots (which would
        // add extra segments, breaking the ACL pattern match).
        let correlation_id = payload
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && !s.contains('.'))
            .ok_or_else(|| {
                SysError::ApiError(
                    "get_messages request missing or invalid correlation_id \
                     (must be non-empty, no dots)"
                        .into(),
                )
            })?;

        let mut data = SessionData::load(session_id);

        // Atomic append-before-read: if the requester provides messages to
        // append, store them first so the returned history includes them.
        if let Some(append_msgs) = payload.get("append_before_read").cloned() {
            let msgs: Vec<Message> = serde_json::from_value(append_msgs)
                .map_err(|e| SysError::ApiError(format!("Failed to parse append_before_read: {e}")))?;
            if !msgs.is_empty() {
                data.messages.extend(msgs);
                data.save(session_id)?;
            }
        }

        // correlation_id is redundant with the scoped topic but retained
        // in the payload for observability (log inspection, debugging).
        let reply_topic = format!("session.v1.response.get_messages.{correlation_id}");
        ipc::publish_json(
            &reply_topic,
            &serde_json::json!({
                "correlation_id": correlation_id,
                "messages": data.messages,
            }),
        )
    }

    /// Handles `session.v1.request.clear` events.
    ///
    /// Creates a new session with `parent_session_id` pointing to the
    /// old one. The old session's data is left intact in KV for history
    /// traversal. Returns the new session ID via a per-request scoped
    /// reply topic (`session.v1.response.clear.<correlation_id>`).
    #[astrid::interceptor("handle_clear")]
    pub fn handle_clear(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let old_session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_SESSION_ID);

        // correlation_id is interpolated into the reply topic as a single
        // segment. Reject empty values and values containing dots (which would
        // add extra segments, breaking the ACL pattern match).
        let correlation_id = payload
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && !s.contains('.'))
            .ok_or_else(|| {
                SysError::ApiError(
                    "clear request missing or invalid correlation_id \
                     (must be non-empty, no dots)"
                        .into(),
                )
            })?;

        let new_session_id = Uuid::new_v4().to_string();

        let new_data = SessionData {
            schema_version: SESSION_DATA_SCHEMA_VERSION,
            parent_session_id: Some(old_session_id.to_string()),
            messages: Vec::new(),
        };
        new_data.save(&new_session_id)?;

        let _ = sys::log(
            "info",
            format!(
                "Session cleared: '{old_session_id}' -> '{new_session_id}' \
                 (old session preserved)"
            ),
        );

        let reply_topic = format!("session.v1.response.clear.{correlation_id}");
        ipc::publish_json(
            &reply_topic,
            &serde_json::json!({
                "correlation_id": correlation_id,
                "new_session_id": new_session_id,
                "old_session_id": old_session_id,
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests (serde-level, no host functions)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Pre-versioning (v0) data with only `messages` deserializes correctly.
    /// The `schema_version` defaults to 0 and `parent_session_id` defaults to None.
    #[test]
    fn test_session_data_v0_backward_compat() {
        let v0_json = r#"{"messages":[]}"#;
        let data: SessionData = serde_json::from_str(v0_json).unwrap();
        assert_eq!(data.schema_version, 0);
        assert!(data.parent_session_id.is_none());
        assert!(data.messages.is_empty());
    }

    /// Current v1 data with all fields round-trips correctly.
    #[test]
    fn test_session_data_v1_round_trip() {
        let data = SessionData {
            schema_version: 1,
            parent_session_id: Some("old-session-abc".into()),
            messages: Vec::new(),
        };
        let json = serde_json::to_string(&data).unwrap();
        let loaded: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.parent_session_id.as_deref(), Some("old-session-abc"));
    }

    /// Unknown future version deserializes without error (serde doesn't
    /// know about version semantics - the `load()` function handles that).
    #[test]
    fn test_session_data_future_version_deserializes() {
        let future_json = r#"{"schema_version":99,"messages":[],"extra_field":"ignored"}"#;
        let data: SessionData = serde_json::from_str(future_json).unwrap();
        assert_eq!(data.schema_version, 99);
        assert!(data.messages.is_empty());
    }

    /// Default SessionData has the current schema version.
    #[test]
    fn test_session_data_default_has_current_version() {
        let data = SessionData::default();
        assert_eq!(data.schema_version, SESSION_DATA_SCHEMA_VERSION);
        assert!(data.parent_session_id.is_none());
    }

    /// v0 data migrates to current version and signals needs_save.
    #[test]
    fn test_migrate_v0_stamps_to_current() {
        let v0_json = r#"{"messages":[]}"#;
        let data: SessionData = serde_json::from_str(v0_json).unwrap();
        assert_eq!(data.schema_version, 0);

        let (migrated, needs_save) = data.migrate().expect("v0 should migrate successfully");
        assert_eq!(migrated.schema_version, SESSION_DATA_SCHEMA_VERSION);
        assert!(needs_save, "v0 -> v1 migration should signal needs_save");
        assert!(migrated.messages.is_empty());
    }

    /// Current version data passes through migrate unchanged.
    #[test]
    fn test_migrate_current_version_is_noop() {
        let data = SessionData::default();
        let (migrated, needs_save) = data.migrate().expect("current version should pass");
        assert_eq!(migrated.schema_version, SESSION_DATA_SCHEMA_VERSION);
        assert!(!needs_save, "current version should not signal needs_save");
    }

    /// Unknown future version fails migration (fail secure).
    #[test]
    fn test_migrate_unknown_version_fails_secure() {
        let data = SessionData {
            schema_version: 99,
            parent_session_id: Some("old".into()),
            messages: Vec::new(),
        };
        let fresh = data.migrate().expect_err("unknown version should fail migration");
        assert_eq!(fresh.schema_version, SESSION_DATA_SCHEMA_VERSION);
        assert!(fresh.parent_session_id.is_none(), "fresh default has no parent");
    }

    /// v0 data with existing messages preserves them through migration.
    #[test]
    fn test_migrate_v0_preserves_messages() {
        let v0_json = r#"{"messages":[{"role":"user","content":"hello"}]}"#;
        let data: SessionData = serde_json::from_str(v0_json).unwrap();
        assert_eq!(data.schema_version, 0);
        assert_eq!(data.messages.len(), 1);

        let (migrated, _) = data.migrate().expect("v0 should migrate");
        assert_eq!(migrated.messages.len(), 1);
    }

    /// v0 data with parent_session_id preserves it through migration.
    #[test]
    fn test_migrate_v0_preserves_parent() {
        let v0_json = r#"{"messages":[],"parent_session_id":"parent-abc"}"#;
        let data: SessionData = serde_json::from_str(v0_json).unwrap();
        let (migrated, _) = data.migrate().expect("v0 should migrate");
        assert_eq!(migrated.parent_session_id.as_deref(), Some("parent-abc"));
    }

    // -- correlation_id validation (scoped reply topic safety) --

    /// Helper replicating the correlation_id validation predicate used in
    /// `handle_get_messages` and `handle_clear`.
    fn valid_correlation_id(payload: &serde_json::Value) -> Option<&str> {
        payload
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && !s.contains('.'))
    }

    #[test]
    fn test_correlation_id_rejects_empty() {
        let payload = serde_json::json!({ "correlation_id": "" });
        assert!(valid_correlation_id(&payload).is_none());
    }

    #[test]
    fn test_correlation_id_rejects_missing() {
        let payload = serde_json::json!({});
        assert!(valid_correlation_id(&payload).is_none());
    }

    #[test]
    fn test_correlation_id_rejects_dots() {
        let payload = serde_json::json!({ "correlation_id": "abc.def" });
        assert!(valid_correlation_id(&payload).is_none());
    }

    #[test]
    fn test_correlation_id_accepts_uuid() {
        let payload =
            serde_json::json!({ "correlation_id": "550e8400-e29b-41d4-a716-446655440000" });
        assert_eq!(
            valid_correlation_id(&payload),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }
}
