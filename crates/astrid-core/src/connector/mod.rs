//! Connector abstraction — unified types for frontends, plugins, and bridges.
//!
//! A **connector** is any component that can send or receive messages on behalf
//! of the Astrid runtime. The three current flavours are:
//!
//! | Source | Example |
//! |--------|---------|
//! | [`ConnectorSource::Native`] | CLI, Discord, Web frontends |
//! | [`ConnectorSource::Wasm`] | WASM plugin providing a tool |
//! | [`ConnectorSource::OpenClaw`] | OpenClaw-bridged plugin |
//!
//! # Adapter traits
//!
//! Four narrow traits describe what a connector *can do*:
//!
//! - [`InboundAdapter`] — produce messages (e.g. user typing in Discord).
//! - [`OutboundAdapter`] — consume messages (e.g. send a reply).
//! - [`ApprovalAdapter`] — ask a human for approval.
//! - [`ElicitationAdapter`] — ask a human for structured input.
//!
//! Blanket implementations bridge the existing [`Frontend`](crate::frontend::Frontend)
//! trait to [`ApprovalAdapter`] and [`ElicitationAdapter`] so every frontend is
//! automatically an adapter with zero migration cost.

// ---------------------------------------------------------------------------

/// Error types for connectors.
pub mod error;
/// Trait definitions for connectors.
pub mod traits;
/// Core types for connectors.
pub mod types;

pub use error::{ConnectorError, ConnectorResult};
pub use traits::*;
pub use types::*;

// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::FrontendType;
    use std::str::FromStr;
    use uuid::Uuid;

    // -- ConnectorId --

    #[test]
    fn connector_id_uniqueness() {
        let a = ConnectorId::new();
        let b = ConnectorId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn connector_id_display_matches_uuid() {
        let uuid = Uuid::new_v4();
        let id = ConnectorId::from_uuid(uuid);
        assert_eq!(id.to_string(), uuid.to_string());
    }

    #[test]
    fn connector_id_roundtrip_serde() {
        let id = ConnectorId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: ConnectorId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    // -- ConnectorCapabilities --

    #[test]
    fn capabilities_full() {
        let c = ConnectorCapabilities::full();
        assert!(c.can_receive);
        assert!(c.can_send);
        assert!(c.can_approve);
        assert!(c.can_elicit);
        assert!(c.supports_rich_media);
        assert!(c.supports_threads);
        assert!(c.supports_buttons);
    }

    #[test]
    fn capabilities_notify_only() {
        let c = ConnectorCapabilities::notify_only();
        assert!(!c.can_receive);
        assert!(c.can_send);
        assert!(!c.can_approve);
    }

    #[test]
    fn capabilities_receive_only() {
        let c = ConnectorCapabilities::receive_only();
        assert!(c.can_receive);
        assert!(!c.can_send);
        assert!(!c.can_approve);
    }

    #[test]
    fn capabilities_default_all_false() {
        let c = ConnectorCapabilities::default();
        assert!(!c.can_receive);
        assert!(!c.can_send);
        assert!(!c.can_approve);
        assert!(!c.can_elicit);
        assert!(!c.supports_rich_media);
        assert!(!c.supports_threads);
        assert!(!c.supports_buttons);
    }

    #[test]
    fn capabilities_serde_roundtrip() {
        let c = ConnectorCapabilities::full();
        let json = serde_json::to_string(&c).unwrap();
        let back: ConnectorCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    // -- ConnectorProfile --

    #[test]
    fn profile_display() {
        assert_eq!(ConnectorProfile::Chat.to_string(), "chat");
        assert_eq!(ConnectorProfile::Interactive.to_string(), "interactive");
        assert_eq!(ConnectorProfile::Notify.to_string(), "notify");
        assert_eq!(ConnectorProfile::Bridge.to_string(), "bridge");
    }

    // -- ConnectorSource --

    #[test]
    fn source_display() {
        assert_eq!(ConnectorSource::Native.to_string(), "native");
        assert_eq!(
            ConnectorSource::Wasm {
                capsule_id: "foo".into()
            }
            .to_string(),
            "wasm(foo)"
        );
        assert_eq!(
            ConnectorSource::OpenClaw {
                capsule_id: "bar".into()
            }
            .to_string(),
            "openclaw(bar)"
        );
    }

    #[test]
    fn source_display_truncates_long_capsule_id() {
        let long_id = "a".repeat(128);
        let src = ConnectorSource::Wasm {
            capsule_id: long_id,
        };
        let display = src.to_string();
        // 64 chars of 'a' + "wasm(" + ")" = 70
        assert_eq!(display.len(), 70);
    }

    #[test]
    fn source_new_wasm_valid() {
        let src = ConnectorSource::new_wasm("my-plugin-1").unwrap();
        assert_eq!(
            src,
            ConnectorSource::Wasm {
                capsule_id: "my-plugin-1".into()
            }
        );
    }

    #[test]
    fn source_new_openclaw_valid() {
        let src = ConnectorSource::new_openclaw("bridge-42").unwrap();
        assert_eq!(
            src,
            ConnectorSource::OpenClaw {
                capsule_id: "bridge-42".into()
            }
        );
    }

    #[test]
    fn source_new_wasm_rejects_empty() {
        let err = ConnectorSource::new_wasm("").unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidPluginId(_)));
    }

    #[test]
    fn source_new_wasm_rejects_uppercase() {
        let err = ConnectorSource::new_wasm("MyPlugin").unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidPluginId(_)));
    }

    #[test]
    fn source_new_wasm_rejects_leading_hyphen() {
        let err = ConnectorSource::new_wasm("-bad").unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidPluginId(_)));
    }

    #[test]
    fn source_new_wasm_rejects_trailing_hyphen() {
        let err = ConnectorSource::new_wasm("bad-").unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidPluginId(_)));
    }

    #[test]
    fn source_new_wasm_rejects_special_chars() {
        let err = ConnectorSource::new_wasm("path/../traversal").unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidPluginId(_)));
    }

    #[test]
    fn source_serde_roundtrip_native() {
        let src = ConnectorSource::Native;
        let json = serde_json::to_string(&src).unwrap();
        let back: ConnectorSource = serde_json::from_str(&json).unwrap();
        assert_eq!(src, back);
    }

    #[test]
    fn source_serde_roundtrip_wasm() {
        let src = ConnectorSource::new_wasm("test-plugin").unwrap();
        let json = serde_json::to_string(&src).unwrap();
        let back: ConnectorSource = serde_json::from_str(&json).unwrap();
        assert_eq!(src, back);
    }

    #[test]
    fn source_serde_roundtrip_openclaw() {
        let src = ConnectorSource::new_openclaw("bridge-1").unwrap();
        let json = serde_json::to_string(&src).unwrap();
        let back: ConnectorSource = serde_json::from_str(&json).unwrap();
        assert_eq!(src, back);
    }

    // -- ConnectorDescriptor --

    #[test]
    fn descriptor_builder() {
        let desc = ConnectorDescriptor::builder("discord-bot", FrontendType::Discord)
            .source(ConnectorSource::Native)
            .capabilities(ConnectorCapabilities::full())
            .profile(ConnectorProfile::Chat)
            .metadata("version", "1.0")
            .build();

        assert_eq!(desc.name, "discord-bot");
        assert_eq!(desc.frontend_type, FrontendType::Discord);
        assert_eq!(desc.source, ConnectorSource::Native);
        assert_eq!(desc.capabilities, ConnectorCapabilities::full());
        assert_eq!(desc.profile, ConnectorProfile::Chat);
        assert_eq!(desc.metadata.get("version").unwrap(), "1.0");
    }

    #[test]
    fn descriptor_serde_roundtrip() {
        let desc = ConnectorDescriptor::builder("cli", FrontendType::Cli)
            .capabilities(ConnectorCapabilities::full())
            .build();

        let json = serde_json::to_string(&desc).unwrap();
        let back: ConnectorDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(desc, back);
    }

    #[test]
    fn descriptor_builder_defaults() {
        let desc = ConnectorDescriptor::builder("minimal", FrontendType::Cli).build();
        assert_eq!(desc.profile, ConnectorProfile::Chat);
        assert_eq!(desc.capabilities, ConnectorCapabilities::default());
        assert_eq!(desc.source, ConnectorSource::Native);
        assert!(desc.metadata.is_empty());
    }

    // -- InboundMessage --

    #[test]
    fn inbound_message_builder() {
        let id = ConnectorId::new();
        let msg = InboundMessage::builder(id, FrontendType::Discord, "user123", "hello")
            .context(serde_json::json!({"key": "value"}))
            .thread_id("thread-1")
            .build();

        assert_eq!(msg.connector_id, id);
        assert_eq!(msg.platform_user_id, "user123");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.context["key"], "value");
        assert_eq!(msg.thread_id.as_deref(), Some("thread-1"));
    }

    #[test]
    fn inbound_message_serde_roundtrip() {
        let id = ConnectorId::new();
        let msg = InboundMessage::builder(id, FrontendType::Discord, "user1", "test")
            .context(serde_json::json!({"nested": {"deep": [1, 2, 3]}}))
            .build();

        let json = serde_json::to_string(&msg).unwrap();
        let back: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.connector_id, id);
        assert_eq!(back.context["nested"]["deep"][1], 2);
    }

    #[test]
    fn inbound_message_empty_content() {
        let id = ConnectorId::new();
        let msg = InboundMessage::builder(id, FrontendType::Cli, "", "").build();
        assert!(msg.platform_user_id.is_empty());
        assert!(msg.content.is_empty());
    }

    // -- OutboundMessage --

    #[test]
    fn outbound_message_builder() {
        let cid = ConnectorId::new();
        let msg = OutboundMessage::builder(cid, "target-user", "response")
            .thread_id("thread-1")
            .reply_to("msg-42")
            .build();

        assert_eq!(msg.connector_id, cid);
        assert_eq!(msg.target_user_id, "target-user");
        assert_eq!(msg.content, "response");
        assert_eq!(msg.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(msg.reply_to.as_deref(), Some("msg-42"));
    }

    #[test]
    fn outbound_message_serde_roundtrip() {
        let cid = ConnectorId::new();
        let msg = OutboundMessage::builder(cid, "user-1", "hello")
            .reply_to("prev-msg")
            .build();

        let json = serde_json::to_string(&msg).unwrap();
        let back: OutboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.connector_id, cid);
        assert_eq!(back.target_user_id, "user-1");
        assert_eq!(back.reply_to.as_deref(), Some("prev-msg"));
    }

    // -- ConnectorError --

    #[test]
    fn error_display() {
        let e = ConnectorError::NotConnected;
        assert_eq!(e.to_string(), "connector not connected");

        let e = ConnectorError::SendFailed("timeout".into());
        assert_eq!(e.to_string(), "send failed: timeout");

        let e = ConnectorError::UnsupportedOperation("rich_media".into());
        assert_eq!(e.to_string(), "unsupported operation: rich_media");

        let e = ConnectorError::InvalidPluginId("bad".into());
        assert_eq!(e.to_string(), "invalid plugin id: bad");
    }

    // --- ConnectorProfile::FromStr ---

    #[test]
    fn profile_from_str_all_variants() {
        assert_eq!(
            ConnectorProfile::from_str("chat").unwrap(),
            ConnectorProfile::Chat
        );
        assert_eq!(
            ConnectorProfile::from_str("interactive").unwrap(),
            ConnectorProfile::Interactive
        );
        assert_eq!(
            ConnectorProfile::from_str("notify").unwrap(),
            ConnectorProfile::Notify
        );
        assert_eq!(
            ConnectorProfile::from_str("bridge").unwrap(),
            ConnectorProfile::Bridge
        );
    }

    #[test]
    fn profile_from_str_case_insensitive() {
        assert_eq!(
            ConnectorProfile::from_str("Chat").unwrap(),
            ConnectorProfile::Chat
        );
        assert_eq!(
            ConnectorProfile::from_str("NOTIFY").unwrap(),
            ConnectorProfile::Notify
        );
    }

    #[test]
    fn profile_from_str_unknown_returns_error() {
        let err = ConnectorProfile::from_str("bogus").unwrap_err();
        assert!(err.contains("unknown connector profile"));
    }
}
