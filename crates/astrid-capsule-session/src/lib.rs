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

use astrid_events::llm::Message;
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// KV key prefix for session data.
const SESSION_KEY_PREFIX: &str = "session.data";

/// Default session ID.
const DEFAULT_SESSION_ID: &str = "default";

/// Build the KV key for a session's data.
fn session_key(session_id: &str) -> String {
    format!("{SESSION_KEY_PREFIX}.{session_id}")
}

/// Persistent conversation session data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SessionData {
    /// Clean conversation message history.
    messages: Vec<Message>,
}

impl SessionData {
    /// Load session data from KV, or create default if not present.
    fn load(session_id: &str) -> Self {
        let key = session_key(session_id);
        kv::get_json::<Self>(&key).unwrap_or_else(|e| {
            let _ = sys::log(
                "error",
                format!("Failed to load session data, starting fresh: {e}"),
            );
            Self::default()
        })
    }

    /// Persist session data to KV.
    fn save(&self, session_id: &str) -> Result<(), SysError> {
        let key = session_key(session_id);
        kv::set_json(&key, self)
    }
}

/// Session capsule. Dumb store.
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
    /// Returns the conversation history to the requester via
    /// `session.response.get_messages`, echoing the correlation ID.
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

        let correlation_id = match payload.get("correlation_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                let _ = sys::log(
                    "warn",
                    "get_messages request missing correlation_id - response may not be routable",
                );
                ""
            },
        };

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

        ipc::publish_json(
            "session.v1.response.get_messages",
            &serde_json::json!({
                "correlation_id": correlation_id,
                "messages": data.messages,
            }),
        )
    }
}
