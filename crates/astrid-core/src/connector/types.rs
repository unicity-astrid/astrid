use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::fmt;
use std::str::FromStr;
use std::collections::HashMap;
use chrono::{DateTime, Utc};

use crate::identity::FrontendType;
use crate::frontend::{Attachment, ElicitationRequest, ElicitationResponse, ApprovalRequest, ApprovalDecision};
use super::error::{ConnectorError, ConnectorResult};

// Limits
// ---------------------------------------------------------------------------

/// Maximum number of connectors a single plugin may register.
///
/// Enforced by the WASM host, the MCP notification handler, and the
/// `McpPlugin` drain. All three must use this constant to stay in sync.
pub const MAX_CONNECTORS_PER_PLUGIN: usize = 32;

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

impl FromStr for ConnectorProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "chat" => Ok(Self::Chat),
            "interactive" => Ok(Self::Interactive),
            "notify" => Ok(Self::Notify),
            "bridge" => Ok(Self::Bridge),
            other => Err(format!("unknown connector profile: {other}")),
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
