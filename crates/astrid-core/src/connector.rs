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

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::error::SecurityError;
use crate::frontend::{
    ApprovalDecision, ApprovalRequest, Attachment, ElicitationRequest, ElicitationResponse,
};
use crate::identity::FrontendType;

// ---------------------------------------------------------------------------
// ConnectorId
// ---------------------------------------------------------------------------

/// Unique, opaque identifier for a registered connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectorId(Uuid);

impl ConnectorId {
    /// Create a new random connector ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing [`Uuid`].
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Return the inner [`Uuid`].
    #[must_use]
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

/// Generates a random ID — equivalent to [`ConnectorId::new`].
///
/// This exists for derive convenience; be aware that each call produces a
/// unique random identifier, not a sentinel/zero value.
impl Default for ConnectorId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConnectorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// ConnectorCapabilities
// ---------------------------------------------------------------------------

/// Declares what a connector is able to do.
///
/// Every flag defaults to `false`; use the convenience constructors
/// ([`full`](Self::full), [`notify_only`](Self::notify_only),
/// [`receive_only`](Self::receive_only)) for common presets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ConnectorCapabilities {
    /// Can receive inbound messages from users.
    pub can_receive: bool,
    /// Can send outbound messages to users.
    pub can_send: bool,
    /// Can present approval requests to a human.
    pub can_approve: bool,
    /// Can present elicitation requests to a human.
    pub can_elicit: bool,
    /// Supports rich media (images, embeds, etc.).
    pub supports_rich_media: bool,
    /// Supports threaded conversations.
    pub supports_threads: bool,
    /// Supports interactive buttons / action rows.
    pub supports_buttons: bool,
}

impl ConnectorCapabilities {
    /// All capabilities enabled.
    #[must_use]
    pub fn full() -> Self {
        Self {
            can_receive: true,
            can_send: true,
            can_approve: true,
            can_elicit: true,
            supports_rich_media: true,
            supports_threads: true,
            supports_buttons: true,
        }
    }

    /// Send-only — for notification bots, webhooks, etc.
    #[must_use]
    pub fn notify_only() -> Self {
        Self {
            can_receive: false,
            can_send: true,
            can_approve: false,
            can_elicit: false,
            supports_rich_media: false,
            supports_threads: false,
            supports_buttons: false,
        }
    }

    /// Receive-only — for ingestion connectors that consume but never reply.
    #[must_use]
    pub fn receive_only() -> Self {
        Self {
            can_receive: true,
            can_send: false,
            can_approve: false,
            can_elicit: false,
            supports_rich_media: false,
            supports_threads: false,
            supports_buttons: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectorProfile
// ---------------------------------------------------------------------------

/// High-level behavioural profile of a connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorProfile {
    /// Full chat interface (CLI, Discord, Slack).
    Chat,
    /// Interactive but not chat-based (Web dashboard, IDE panel).
    Interactive,
    /// Fire-and-forget notifications only.
    Notify,
    /// Protocol bridge (`OpenClaw`, MCP relay).
    Bridge,
}

impl fmt::Display for ConnectorProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Interactive => write!(f, "interactive"),
            Self::Notify => write!(f, "notify"),
            Self::Bridge => write!(f, "bridge"),
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectorSource
// ---------------------------------------------------------------------------

/// Where a connector originates from.
///
/// # Trust boundary
///
/// The [`new_wasm`](Self::new_wasm) and [`new_openclaw`](Self::new_openclaw)
/// constructors validate the `plugin_id`. Direct struct construction or
/// [`Deserialize`] bypass this validation — only use those paths with
/// trusted data.
///
/// # Serialization
///
/// Uses serde's default externally-tagged representation:
/// - `"native"` for [`Native`](Self::Native)
/// - `{"wasm": {"plugin_id": "..."}}` for [`Wasm`](Self::Wasm)
/// - `{"open_claw": {"plugin_id": "..."}}` for [`OpenClaw`](Self::OpenClaw)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorSource {
    /// Built-in frontend (CLI, Discord, Web).
    Native,
    /// WASM plugin providing a connector.
    Wasm {
        /// Plugin identifier — lowercase alphanumeric and hyphens, must not
        /// start or end with a hyphen. Validated by
        /// [`ConnectorSource::new_wasm`]; the canonical `PluginId` type
        /// lives in `astrid-plugins`.
        plugin_id: String,
    },
    /// `OpenClaw`-bridged plugin connector.
    OpenClaw {
        /// Plugin identifier — lowercase alphanumeric and hyphens, must not
        /// start or end with a hyphen. Validated by
        /// [`ConnectorSource::new_openclaw`]; the canonical `PluginId` type
        /// lives in `astrid-plugins`.
        plugin_id: String,
    },
}

impl ConnectorSource {
    /// Create a [`Wasm`](Self::Wasm) source with a validated plugin ID.
    ///
    /// The `plugin_id` must be non-empty, contain only lowercase ASCII
    /// alphanumeric characters and hyphens, and must not start or end with
    /// a hyphen (the same rules enforced by `PluginId` in `astrid-plugins`).
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::InvalidPluginId`] if the ID is empty,
    /// starts or ends with a hyphen, or contains characters outside
    /// `[a-z0-9-]`.
    pub fn new_wasm(plugin_id: impl Into<String>) -> ConnectorResult<Self> {
        let id = plugin_id.into();
        validate_plugin_id(&id)?;
        Ok(Self::Wasm { plugin_id: id })
    }

    /// Create an [`OpenClaw`](Self::OpenClaw) source with a validated plugin ID.
    ///
    /// The `plugin_id` must be non-empty, contain only lowercase ASCII
    /// alphanumeric characters and hyphens, and must not start or end with
    /// a hyphen (the same rules enforced by `PluginId` in `astrid-plugins`).
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::InvalidPluginId`] if the ID is empty,
    /// starts or ends with a hyphen, or contains characters outside
    /// `[a-z0-9-]`.
    pub fn new_openclaw(plugin_id: impl Into<String>) -> ConnectorResult<Self> {
        let id = plugin_id.into();
        validate_plugin_id(&id)?;
        Ok(Self::OpenClaw { plugin_id: id })
    }
}

/// Validate that a plugin ID is non-empty, contains only `[a-z0-9-]`, and
/// does not start or end with a hyphen. Mirrors the rules in
/// `PluginId::validate` from `astrid-plugins`.
fn validate_plugin_id(id: &str) -> ConnectorResult<()> {
    if id.is_empty() {
        return Err(ConnectorError::InvalidPluginId(
            "plugin_id must not be empty".into(),
        ));
    }
    let first = id.as_bytes()[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(ConnectorError::InvalidPluginId(format!(
            "plugin_id must start with [a-z0-9], got {id:?}"
        )));
    }
    if id.ends_with('-') {
        return Err(ConnectorError::InvalidPluginId(format!(
            "plugin_id must not end with a hyphen, got {id:?}"
        )));
    }
    if let Some(bad) = id
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
    {
        return Err(ConnectorError::InvalidPluginId(format!(
            "plugin_id contains invalid character {bad:?}"
        )));
    }
    Ok(())
}

impl fmt::Display for ConnectorSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native => write!(f, "native"),
            // Use truncate_to_boundary for UTF-8 safety if deserialization
            // bypasses validation and injects non-ASCII plugin IDs.
            Self::Wasm { plugin_id } => {
                let safe = crate::utils::truncate_to_boundary(plugin_id, 64);
                write!(f, "wasm({safe})")
            },
            Self::OpenClaw { plugin_id } => {
                let safe = crate::utils::truncate_to_boundary(plugin_id, 64);
                write!(f, "openclaw({safe})")
            },
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectorDescriptor
// ---------------------------------------------------------------------------

/// Immutable description of a registered connector.
///
/// Created via the builder pattern — call [`ConnectorDescriptor::builder`] to
/// start.
///
/// # Trust boundary
///
/// The `id` and `registered_at` fields are server-assigned (generated in
/// [`ConnectorDescriptorBuilder::build`]). This type derives [`Deserialize`]
/// for trusted persistence (e.g. `SurrealDB`). **Do not** deserialize from
/// untrusted sources without post-deserialization validation — a forged `id`
/// could allow connector impersonation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorDescriptor {
    /// Unique connector identity.
    pub id: ConnectorId,
    /// Human-readable name.
    pub name: String,
    /// The platform type this connector serves.
    pub frontend_type: FrontendType,
    /// Where the connector comes from.
    pub source: ConnectorSource,
    /// What the connector can do.
    pub capabilities: ConnectorCapabilities,
    /// Behavioural profile.
    pub profile: ConnectorProfile,
    /// When this connector was registered.
    pub registered_at: DateTime<Utc>,
    /// Arbitrary key-value metadata.
    pub metadata: HashMap<String, String>,
}

/// Builder for [`ConnectorDescriptor`].
#[derive(Debug)]
pub struct ConnectorDescriptorBuilder {
    name: String,
    frontend_type: FrontendType,
    source: ConnectorSource,
    capabilities: ConnectorCapabilities,
    profile: ConnectorProfile,
    metadata: HashMap<String, String>,
}

impl ConnectorDescriptor {
    /// Start building a new descriptor.
    #[must_use]
    pub fn builder(
        name: impl Into<String>,
        frontend_type: FrontendType,
    ) -> ConnectorDescriptorBuilder {
        ConnectorDescriptorBuilder {
            name: name.into(),
            frontend_type,
            source: ConnectorSource::Native,
            capabilities: ConnectorCapabilities::default(),
            profile: ConnectorProfile::Chat,
            metadata: HashMap::new(),
        }
    }
}

impl ConnectorDescriptorBuilder {
    /// Set the connector source.
    #[must_use]
    pub fn source(mut self, source: ConnectorSource) -> Self {
        self.source = source;
        self
    }

    /// Set the connector capabilities.
    #[must_use]
    pub fn capabilities(mut self, capabilities: ConnectorCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Set the connector profile.
    #[must_use]
    pub fn profile(mut self, profile: ConnectorProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Insert a metadata entry.
    #[must_use]
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Consume the builder and produce a [`ConnectorDescriptor`].
    #[must_use]
    pub fn build(self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            id: ConnectorId::new(),
            name: self.name,
            frontend_type: self.frontend_type,
            source: self.source,
            capabilities: self.capabilities,
            profile: self.profile,
            registered_at: Utc::now(),
            metadata: self.metadata,
        }
    }
}

// ---------------------------------------------------------------------------
// InboundMessage
// ---------------------------------------------------------------------------

/// A message arriving *into* the runtime from a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Which connector produced this message.
    pub connector_id: ConnectorId,
    /// Platform the message originated on.
    pub platform: FrontendType,
    /// Platform-specific user identifier (e.g. Discord snowflake).
    pub platform_user_id: String,
    /// Textual content.
    pub content: String,
    /// Opaque context payload (JSON) for bridge compatibility.
    pub context: serde_json::Value,
    /// Attached files / URLs.
    pub attachments: Vec<Attachment>,
    /// Thread identifier, if threaded.
    pub thread_id: Option<String>,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
}

/// Builder for [`InboundMessage`].
#[derive(Debug)]
pub struct InboundMessageBuilder {
    connector_id: ConnectorId,
    platform: FrontendType,
    platform_user_id: String,
    content: String,
    context: serde_json::Value,
    attachments: Vec<Attachment>,
    thread_id: Option<String>,
    timestamp: DateTime<Utc>,
}

impl InboundMessage {
    /// Start building a new inbound message.
    #[must_use]
    pub fn builder(
        connector_id: ConnectorId,
        platform: FrontendType,
        platform_user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> InboundMessageBuilder {
        InboundMessageBuilder {
            connector_id,
            platform,
            platform_user_id: platform_user_id.into(),
            content: content.into(),
            context: serde_json::Value::Null,
            attachments: Vec::new(),
            thread_id: None,
            timestamp: Utc::now(),
        }
    }
}

impl InboundMessageBuilder {
    /// Set the opaque context payload.
    #[must_use]
    pub fn context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }

    /// Add an attachment.
    #[must_use]
    pub fn attachment(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Set the thread ID.
    #[must_use]
    pub fn thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Override the timestamp (defaults to now).
    #[must_use]
    pub fn timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Consume the builder and produce an [`InboundMessage`].
    #[must_use]
    pub fn build(self) -> InboundMessage {
        InboundMessage {
            connector_id: self.connector_id,
            platform: self.platform,
            platform_user_id: self.platform_user_id,
            content: self.content,
            context: self.context,
            attachments: self.attachments,
            thread_id: self.thread_id,
            timestamp: self.timestamp,
        }
    }
}

// ---------------------------------------------------------------------------
// OutboundMessage
// ---------------------------------------------------------------------------

/// A message leaving the runtime toward a connector's user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    /// Which connector should deliver this message.
    pub connector_id: ConnectorId,
    /// Target user (Astrid-resolved identity string).
    pub target_user_id: String,
    /// Textual content.
    pub content: String,
    /// Attached files / URLs.
    pub attachments: Vec<Attachment>,
    /// Thread identifier, if threaded.
    pub thread_id: Option<String>,
    /// Message ID this is replying to, if any.
    pub reply_to: Option<String>,
}

/// Builder for [`OutboundMessage`].
#[derive(Debug)]
pub struct OutboundMessageBuilder {
    connector_id: ConnectorId,
    target_user_id: String,
    content: String,
    attachments: Vec<Attachment>,
    thread_id: Option<String>,
    reply_to: Option<String>,
}

impl OutboundMessage {
    /// Start building a new outbound message.
    #[must_use]
    pub fn builder(
        connector_id: ConnectorId,
        target_user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> OutboundMessageBuilder {
        OutboundMessageBuilder {
            connector_id,
            target_user_id: target_user_id.into(),
            content: content.into(),
            attachments: Vec::new(),
            thread_id: None,
            reply_to: None,
        }
    }
}

impl OutboundMessageBuilder {
    /// Add an attachment.
    #[must_use]
    pub fn attachment(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Set the thread ID.
    #[must_use]
    pub fn thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Set the message this is replying to.
    #[must_use]
    pub fn reply_to(mut self, message_id: impl Into<String>) -> Self {
        self.reply_to = Some(message_id.into());
        self
    }

    /// Consume the builder and produce an [`OutboundMessage`].
    #[must_use]
    pub fn build(self) -> OutboundMessage {
        OutboundMessage {
            connector_id: self.connector_id,
            target_user_id: self.target_user_id,
            content: self.content,
            attachments: self.attachments,
            thread_id: self.thread_id,
            reply_to: self.reply_to,
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectorError
// ---------------------------------------------------------------------------

/// Errors specific to connector operations.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    /// The connector is not connected or has been unregistered.
    #[error("connector not connected")]
    NotConnected,

    /// Sending a message failed.
    #[error("send failed: {0}")]
    SendFailed(String),

    /// The plugin ID failed validation (must be non-empty, lowercase
    /// alphanumeric and hyphens, must not start or end with a hyphen).
    #[error("invalid plugin id: {0}")]
    InvalidPluginId(String),

    /// The requested operation is not supported by this connector.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// Serialization / deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// An underlying security error.
    #[error(transparent)]
    Security(#[from] SecurityError),

    /// Catch-all for internal errors.
    #[error("internal connector error: {0}")]
    Internal(String),
}

/// Convenience alias for connector operations.
pub type ConnectorResult<T> = Result<T, ConnectorError>;

// ---------------------------------------------------------------------------
// Adapter traits
// ---------------------------------------------------------------------------

/// Produces inbound messages from an external source.
///
/// Call [`subscribe`](Self::subscribe) to obtain a channel receiver that
/// yields [`InboundMessage`]s as they arrive.
///
/// # Single-subscriber semantics
///
/// This is a **single-subscriber** adapter. The first call to `subscribe`
/// creates the internal channel and returns the [`mpsc::Receiver`]. Subsequent
/// calls should return [`ConnectorError::UnsupportedOperation`] — the adapter
/// holds the `Sender` half internally. If the `Receiver` is dropped, inflight
/// sends will fail and the adapter may treat the subscriber as disconnected.
#[async_trait]
pub trait InboundAdapter: Send + Sync {
    /// Subscribe to inbound messages.
    ///
    /// Returns the receive half of an internal `mpsc` channel. May only be
    /// called once; subsequent calls should fail with
    /// [`ConnectorError::UnsupportedOperation`].
    async fn subscribe(&self) -> ConnectorResult<mpsc::Receiver<InboundMessage>>;
}

/// Sends outbound messages to an external destination.
#[async_trait]
pub trait OutboundAdapter: Send + Sync {
    /// Send a message through this connector.
    async fn send(&self, message: OutboundMessage) -> ConnectorResult<()>;
}

/// Presents approval requests to a human decision-maker.
#[async_trait]
pub trait ApprovalAdapter: Send + Sync {
    /// Request human approval for an operation.
    async fn request_approval(&self, request: ApprovalRequest)
    -> ConnectorResult<ApprovalDecision>;
}

/// Presents elicitation requests to a human for structured input.
#[async_trait]
pub trait ElicitationAdapter: Send + Sync {
    /// Elicit structured input from a human.
    async fn elicit(&self, request: ElicitationRequest) -> ConnectorResult<ElicitationResponse>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
                plugin_id: "foo".into()
            }
            .to_string(),
            "wasm(foo)"
        );
        assert_eq!(
            ConnectorSource::OpenClaw {
                plugin_id: "bar".into()
            }
            .to_string(),
            "openclaw(bar)"
        );
    }

    #[test]
    fn source_display_truncates_long_plugin_id() {
        let long_id = "a".repeat(128);
        let src = ConnectorSource::Wasm { plugin_id: long_id };
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
                plugin_id: "my-plugin-1".into()
            }
        );
    }

    #[test]
    fn source_new_openclaw_valid() {
        let src = ConnectorSource::new_openclaw("bridge-42").unwrap();
        assert_eq!(
            src,
            ConnectorSource::OpenClaw {
                plugin_id: "bridge-42".into()
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
    fn error_from_security_error() {
        let sec = SecurityError::Internal("boom".into());
        let conn: ConnectorError = ConnectorError::from(sec);
        assert!(matches!(conn, ConnectorError::Security(_)));
    }

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
}
