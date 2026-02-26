//! Discord Gateway protocol types, opcodes, and intent flags.
//!
//! Implements the wire format for the Discord Gateway v10 protocol.
//! Only the opcodes and fields required by the proxy are modelled;
//! dispatch event payloads are forwarded as opaque `serde_json::Value`.

use serde::{Deserialize, Serialize};

// ── Opcodes ──────────────────────────────────────────────────

/// Discord Gateway opcodes.
pub(crate) mod opcode {
    /// Event dispatch (receive only).
    pub(crate) const DISPATCH: u8 = 0;
    /// Heartbeat (bidirectional).
    pub(crate) const HEARTBEAT: u8 = 1;
    /// Identify (send only).
    pub(crate) const IDENTIFY: u8 = 2;
    /// Resume (send only).
    pub(crate) const RESUME: u8 = 6;
    /// Server requests reconnect (receive only).
    pub(crate) const RECONNECT: u8 = 7;
    /// Invalid session (receive only).
    pub(crate) const INVALID_SESSION: u8 = 9;
    /// Hello — contains heartbeat interval (receive only).
    pub(crate) const HELLO: u8 = 10;
    /// Heartbeat ACK (receive only).
    pub(crate) const HEARTBEAT_ACK: u8 = 11;
}

/// Close codes that indicate a fatal, non-recoverable error.
pub(crate) mod close_code {
    /// Authentication failed — bad token.
    pub(crate) const AUTHENTICATION_FAILED: u16 = 4004;
    /// Invalid shard configuration.
    pub(crate) const INVALID_SHARD: u16 = 4010;
    /// Invalid intents value.
    pub(crate) const INVALID_INTENTS: u16 = 4013;
    /// Disallowed intents (not enabled in portal).
    pub(crate) const DISALLOWED_INTENTS: u16 = 4014;
}

// ── Intent Flags ─────────────────────────────────────────────

/// Default Gateway intents bitmask.
///
/// `GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES`
///
/// `MESSAGE_CONTENT` (1 << 15) is **not** included by default — it is a
/// privileged intent that must be opted in via config and enabled in the
/// Discord Developer Portal.
pub(crate) const DEFAULT_INTENTS: u32 = (1 << 0) | (1 << 9) | (1 << 12);

// ── Wire Types ───────────────────────────────────────────────

/// Raw Gateway payload as received/sent over `WebSocket`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GatewayPayload {
    /// Opcode for the payload.
    pub op: u8,
    /// Event data (opcode-dependent).
    #[serde(default)]
    pub d: Option<serde_json::Value>,
    /// Sequence number (only for `op=0` dispatch events).
    #[serde(default)]
    pub s: Option<u64>,
    /// Event name (only for `op=0` dispatch events).
    #[serde(default)]
    pub t: Option<String>,
}

/// Hello payload (`op=10`).
#[derive(Debug, Deserialize)]
pub(crate) struct HelloPayload {
    /// Heartbeat interval in milliseconds.
    pub heartbeat_interval: u64,
}

/// Ready event data (`t="READY"`).
#[derive(Debug, Deserialize)]
pub(crate) struct ReadyPayload {
    /// Session ID for resuming.
    pub session_id: String,
    /// Preferred resume gateway URL.
    pub resume_gateway_url: String,
    /// The bot user object.
    pub user: ReadyUser,
}

/// User object from the READY event.
#[derive(Debug, Deserialize)]
pub(crate) struct ReadyUser {
    /// The bot's user ID.
    pub id: String,
}

/// Response from `GET /gateway/bot`.
#[derive(Debug, Deserialize)]
pub(crate) struct GatewayBotResponse {
    /// Gateway `WebSocket` URL.
    pub url: String,
}

// ── Identify / Resume Payloads ───────────────────────────────

/// Build an Identify payload (`op=2`).
pub(crate) fn build_identify(token: &str, intents: u32) -> GatewayPayload {
    GatewayPayload {
        op: opcode::IDENTIFY,
        d: Some(serde_json::json!({
            "token": token,
            "intents": intents,
            "properties": {
                "os": std::env::consts::OS,
                "browser": "astrid",
                "device": "astrid",
            },
        })),
        s: None,
        t: None,
    }
}

/// Build a Resume payload (`op=6`).
pub(crate) fn build_resume(token: &str, session_id: &str, sequence: u64) -> GatewayPayload {
    GatewayPayload {
        op: opcode::RESUME,
        d: Some(serde_json::json!({
            "token": token,
            "session_id": session_id,
            "seq": sequence,
        })),
        s: None,
        t: None,
    }
}

/// Build a Heartbeat payload (`op=1`).
pub(crate) fn build_heartbeat(sequence: Option<u64>) -> GatewayPayload {
    GatewayPayload {
        op: opcode::HEARTBEAT,
        d: sequence.map(serde_json::Value::from),
        s: None,
        t: None,
    }
}

/// Returns `true` if the given close code is fatal (non-recoverable).
#[allow(dead_code)]
pub(crate) fn is_fatal_close_code(code: u16) -> bool {
    matches!(
        code,
        close_code::AUTHENTICATION_FAILED
            | close_code::INVALID_SHARD
            | close_code::INVALID_INTENTS
            | close_code::DISALLOWED_INTENTS
    )
}

/// Allowed resume gateway URL domains.
const ALLOWED_RESUME_DOMAINS: &[&str] = &["discord.gg"];

/// Validate that a resume gateway URL is from an allowed domain.
pub(crate) fn is_valid_resume_url(url: &str) -> bool {
    url.starts_with("wss://")
        && ALLOWED_RESUME_DOMAINS.iter().any(|domain| {
            let host_start = "wss://".len();
            let host_part = &url[host_start..];
            // Extract host (before any path/query).
            let host = host_part.split('/').next().unwrap_or("");
            let host = host.split('?').next().unwrap_or("");
            let host = host.split(':').next().unwrap_or("");
            host == *domain || host.ends_with(&format!(".{domain}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_constants() {
        assert_eq!(opcode::DISPATCH, 0);
        assert_eq!(opcode::HEARTBEAT, 1);
        assert_eq!(opcode::IDENTIFY, 2);
        assert_eq!(opcode::RESUME, 6);
        assert_eq!(opcode::RECONNECT, 7);
        assert_eq!(opcode::INVALID_SESSION, 9);
        assert_eq!(opcode::HELLO, 10);
        assert_eq!(opcode::HEARTBEAT_ACK, 11);
    }

    #[test]
    fn close_code_constants() {
        assert_eq!(close_code::AUTHENTICATION_FAILED, 4004);
        assert_eq!(close_code::INVALID_SHARD, 4010);
        assert_eq!(close_code::INVALID_INTENTS, 4013);
        assert_eq!(close_code::DISALLOWED_INTENTS, 4014);
    }

    #[test]
    fn default_intents_value() {
        // GUILDS(1) | GUILD_MESSAGES(512) | DIRECT_MESSAGES(4096) |
        // GUILDS(1) | GUILD_MESSAGES(512) | DIRECT_MESSAGES(4096) = 4609
        assert_eq!(DEFAULT_INTENTS, 1 | 512 | 4096);
        assert_eq!(DEFAULT_INTENTS, 4609);
    }

    #[test]
    fn fatal_close_codes() {
        assert!(is_fatal_close_code(4004));
        assert!(is_fatal_close_code(4010));
        assert!(is_fatal_close_code(4013));
        assert!(is_fatal_close_code(4014));
    }

    #[test]
    fn non_fatal_close_codes() {
        assert!(!is_fatal_close_code(1000));
        assert!(!is_fatal_close_code(1001));
        assert!(!is_fatal_close_code(4000));
        assert!(!is_fatal_close_code(4001));
        assert!(!is_fatal_close_code(4009));
        assert!(!is_fatal_close_code(4011));
        assert!(!is_fatal_close_code(4012));
    }

    #[test]
    fn gateway_payload_roundtrip() {
        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({"key": "value"})),
            s: Some(42),
            t: Some("MESSAGE_CREATE".to_string()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let restored: GatewayPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.op, 0);
        assert_eq!(restored.s, Some(42));
        assert_eq!(restored.t.as_deref(), Some("MESSAGE_CREATE"));
    }

    #[test]
    fn gateway_payload_minimal() {
        let json = r#"{"op":10,"d":{"heartbeat_interval":41250}}"#;
        let payload: GatewayPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.op, opcode::HELLO);
        assert!(payload.s.is_none());
        assert!(payload.t.is_none());

        let hello: HelloPayload = serde_json::from_value(payload.d.unwrap()).unwrap();
        assert_eq!(hello.heartbeat_interval, 41250);
    }

    #[test]
    fn build_identify_payload() {
        let payload = build_identify("Bot testtoken", 37377);
        assert_eq!(payload.op, opcode::IDENTIFY);
        let d = payload.d.unwrap();
        assert_eq!(d["token"], "Bot testtoken");
        assert_eq!(d["intents"], 37377);
        assert_eq!(d["properties"]["browser"], "astrid");
    }

    #[test]
    fn build_resume_payload() {
        let payload = build_resume("Bot tok", "sess-123", 42);
        assert_eq!(payload.op, opcode::RESUME);
        let d = payload.d.unwrap();
        assert_eq!(d["token"], "Bot tok");
        assert_eq!(d["session_id"], "sess-123");
        assert_eq!(d["seq"], 42);
    }

    #[test]
    fn build_heartbeat_with_seq() {
        let payload = build_heartbeat(Some(99));
        assert_eq!(payload.op, opcode::HEARTBEAT);
        assert_eq!(payload.d, Some(serde_json::Value::from(99)));
    }

    #[test]
    fn build_heartbeat_null_seq() {
        let payload = build_heartbeat(None);
        assert_eq!(payload.op, opcode::HEARTBEAT);
        assert!(payload.d.is_none());
    }

    #[test]
    fn valid_resume_urls() {
        assert!(is_valid_resume_url(
            "wss://gateway.discord.gg/?v=10&encoding=json"
        ));
        assert!(is_valid_resume_url("wss://gateway-us-east1-b.discord.gg"));
    }

    #[test]
    fn invalid_resume_urls() {
        assert!(!is_valid_resume_url("ws://gateway.discord.gg"));
        assert!(!is_valid_resume_url("wss://evil.example.com"));
        assert!(!is_valid_resume_url("wss://notdiscord.gg/gateway"));
        assert!(!is_valid_resume_url(""));
        assert!(!is_valid_resume_url("https://gateway.discord.gg"));
        assert!(!is_valid_resume_url("wss://cdn.discord.media/gateway"));
    }

    #[test]
    fn ready_payload_deserializes() {
        let json = serde_json::json!({
            "session_id": "abc123",
            "resume_gateway_url": "wss://gateway.discord.gg",
            "user": { "id": "bot-user-id" },
            "guilds": [],
            "application": { "id": "app-id" }
        });
        let ready: ReadyPayload = serde_json::from_value(json).unwrap();
        assert_eq!(ready.session_id, "abc123");
        assert_eq!(ready.resume_gateway_url, "wss://gateway.discord.gg");
        assert_eq!(ready.user.id, "bot-user-id");
    }

    #[test]
    fn gateway_bot_response_deserializes() {
        let json = serde_json::json!({
            "url": "wss://gateway.discord.gg",
            "shards": 1,
            "session_start_limit": {
                "total": 1000,
                "remaining": 999,
                "reset_after": 14400000,
                "max_concurrency": 1
            }
        });
        let resp: GatewayBotResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.url, "wss://gateway.discord.gg");
    }
}
