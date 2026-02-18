//! MCP client capabilities handlers.
//!
//! These handlers implement client-side capabilities from the MCP Nov 2025 spec:
//! - Sampling: Server-initiated LLM calls
//! - Roots: Server inquiries about operational boundaries
//! - Elicitation: Server requests for user input (canonical types from `astrid-core`)
//! - URL Elicitation: OAuth flows, payments, credentials (canonical types from `astrid-core`)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// Canonical elicitation types from astrid-core (single source of truth).
use astrid_core::{
    ElicitationAction as CoreElicitationAction, ElicitationRequest, ElicitationResponse,
    ElicitationSchema, SelectOption, UrlElicitationRequest, UrlElicitationResponse,
};

/// Request for LLM sampling from a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingRequest {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// Server making the request.
    pub server: String,
    /// Messages to send to the LLM.
    pub messages: Vec<SamplingMessage>,
    /// Optional system prompt.
    pub system: Option<String>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Temperature setting.
    pub temperature: Option<f64>,
    /// Model preference (hint, not requirement).
    pub model_hint: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

/// Message in a sampling request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingMessage {
    /// Role: "user", "assistant", or "system".
    pub role: String,
    /// Message content.
    pub content: SamplingContent,
}

/// Content in a sampling message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SamplingContent {
    /// Text content.
    Text {
        /// The text.
        text: String,
    },
    /// Image content.
    Image {
        /// Base64-encoded image data.
        data: String,
        /// MIME type.
        mime_type: String,
    },
}

/// Response to a sampling request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingResponse {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// Whether the request was successful.
    pub success: bool,
    /// Generated content.
    pub content: Option<String>,
    /// Model used.
    pub model: Option<String>,
    /// Stop reason.
    pub stop_reason: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Handler for server-initiated LLM sampling requests.
#[async_trait]
pub trait SamplingHandler: Send + Sync {
    /// Handle a sampling request from a server.
    ///
    /// The implementation should:
    /// 1. Validate the request is within allowed parameters
    /// 2. Forward to the LLM if authorized
    /// 3. Return the response
    async fn handle_sampling(&self, request: SamplingRequest) -> SamplingResponse;

    /// Check if sampling is enabled for a server.
    fn is_enabled(&self, server: &str) -> bool;

    /// Get the maximum tokens allowed for a server.
    fn max_tokens(&self, server: &str) -> Option<u32>;
}

/// Request for operational boundaries (roots).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootsRequest {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// Server making the request.
    pub server: String,
}

/// Response to a roots request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootsResponse {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// List of root directories/URIs the server can access.
    pub roots: Vec<Root>,
}

/// A root directory or URI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Root {
    /// URI of the root (e.g., `file:///home/user/project`).
    pub uri: String,
    /// Human-readable name.
    pub name: Option<String>,
}

/// Handler for server inquiries about operational boundaries.
#[async_trait]
pub trait RootsHandler: Send + Sync {
    /// Handle a roots request from a server.
    ///
    /// Returns the list of roots (directories, URIs) that the server
    /// is allowed to access.
    async fn handle_roots(&self, request: RootsRequest) -> RootsResponse;
}

// ─── Elicitation handler traits ─────────────────────────────────────────────
//
// The elicitation data types (ElicitationRequest, ElicitationResponse,
// ElicitationSchema, UrlElicitationRequest, UrlElicitationResponse, etc.)
// are canonical types defined in `astrid-core::frontend`. The handler
// traits below use those types directly — no MCP-local duplicates.

/// Handler for server requests for user input.
///
/// Implementations receive canonical [`ElicitationRequest`] from `astrid-core`
/// and should return an [`ElicitationResponse`] after collecting user input.
#[async_trait]
pub trait ElicitationHandler: Send + Sync {
    /// Handle an elicitation request from a server.
    ///
    /// The implementation should:
    /// 1. Display the message to the user
    /// 2. Collect their response based on the schema
    /// 3. Return the appropriate action (submit, cancel, dismiss)
    async fn handle_elicitation(&self, request: ElicitationRequest) -> ElicitationResponse;
}

/// Handler for URL-based elicitation (OAuth, payments).
///
/// Implementations receive canonical [`UrlElicitationRequest`] from `astrid-core`
/// and should return a [`UrlElicitationResponse`] after the user completes the flow.
#[async_trait]
pub trait UrlElicitationHandler: Send + Sync {
    /// Handle a URL elicitation request from a server.
    ///
    /// The implementation should:
    /// 1. Open the URL in the user's browser
    /// 2. Listen for a callback (if OAuth/payment)
    /// 3. Return the result
    ///
    /// IMPORTANT: For payment flows, the LLM should NEVER see the amounts.
    /// The client handles the payment UI directly.
    async fn handle_url_elicitation(
        &self,
        request: UrlElicitationRequest,
    ) -> UrlElicitationResponse;
}

/// Composite handler that combines all capability handlers.
pub struct CapabilitiesHandler {
    /// Sampling handler.
    pub sampling: Option<Box<dyn SamplingHandler>>,
    /// Roots handler.
    pub roots: Option<Box<dyn RootsHandler>>,
    /// Elicitation handler.
    pub elicitation: Option<Box<dyn ElicitationHandler>>,
    /// URL elicitation handler.
    pub url_elicitation: Option<Box<dyn UrlElicitationHandler>>,
}

impl Default for CapabilitiesHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilitiesHandler {
    /// Create an empty capabilities handler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sampling: None,
            roots: None,
            elicitation: None,
            url_elicitation: None,
        }
    }

    /// Set the sampling handler.
    #[must_use]
    pub fn with_sampling(mut self, handler: impl SamplingHandler + 'static) -> Self {
        self.sampling = Some(Box::new(handler));
        self
    }

    /// Set the roots handler.
    #[must_use]
    pub fn with_roots(mut self, handler: impl RootsHandler + 'static) -> Self {
        self.roots = Some(Box::new(handler));
        self
    }

    /// Set the elicitation handler.
    #[must_use]
    pub fn with_elicitation(mut self, handler: impl ElicitationHandler + 'static) -> Self {
        self.elicitation = Some(Box::new(handler));
        self
    }

    /// Set the URL elicitation handler.
    #[must_use]
    pub fn with_url_elicitation(mut self, handler: impl UrlElicitationHandler + 'static) -> Self {
        self.url_elicitation = Some(Box::new(handler));
        self
    }

    /// Check if sampling is available.
    #[must_use]
    pub fn has_sampling(&self) -> bool {
        self.sampling.is_some()
    }

    /// Check if roots is available.
    #[must_use]
    pub fn has_roots(&self) -> bool {
        self.roots.is_some()
    }

    /// Check if elicitation is available.
    #[must_use]
    pub fn has_elicitation(&self) -> bool {
        self.elicitation.is_some()
    }

    /// Check if URL elicitation is available.
    #[must_use]
    pub fn has_url_elicitation(&self) -> bool {
        self.url_elicitation.is_some()
    }
}

impl std::fmt::Debug for CapabilitiesHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapabilitiesHandler")
            .field("sampling", &self.has_sampling())
            .field("roots", &self.has_roots())
            .field("elicitation", &self.has_elicitation())
            .field("url_elicitation", &self.has_url_elicitation())
            .finish()
    }
}

// ─── rmcp ↔ core conversion helpers ─────────────────────────────────────────

use rmcp::model::CreateElicitationRequestParams;

/// Convert an rmcp elicitation schema to a core elicitation schema.
///
/// The rmcp schema is a JSON Schema object with typed properties, while the core
/// schema is a simple enum (`Text`/`Secret`/`Select`/`Confirm`). This does a
/// best-effort conversion based on the first property's type.
///
/// Returns `(core_schema, first_property_name)` where the property name is used
/// to wrap single-value responses back into the object format rmcp expects.
fn convert_rmcp_schema(
    schema: &rmcp::model::ElicitationSchema,
) -> (ElicitationSchema, Option<String>) {
    let first = schema.properties.iter().next();
    let prop_name = first.map(|(name, _)| name.clone());

    if let Some((_, primitive)) = first {
        // Serialize the PrimitiveSchema to JSON to inspect its type without
        // depending on rmcp's internal enum variant structure.
        if let Ok(json) = serde_json::to_value(primitive) {
            let type_str = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match type_str {
                "boolean" => {
                    let default = json
                        .get("default")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    return (ElicitationSchema::Confirm { default }, prop_name);
                },
                "string" => {
                    let placeholder = json
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(String::from);
                    #[allow(clippy::cast_possible_truncation)]
                    let max_length = json
                        .get("maxLength")
                        .and_then(serde_json::Value::as_u64)
                        .map(|m| m as usize);
                    return (
                        ElicitationSchema::Text {
                            placeholder,
                            max_length,
                        },
                        prop_name,
                    );
                },
                _ => {},
            }

            // Check for enum type (no "type" field, has "enum" array)
            if let Some(enum_values) = json.get("enum").and_then(|e| e.as_array()) {
                let options: Vec<SelectOption> = enum_values
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| SelectOption::new(s, s))
                    .collect();
                if !options.is_empty() {
                    return (
                        ElicitationSchema::Select {
                            options,
                            multiple: false,
                        },
                        prop_name,
                    );
                }
            }
        }
    }

    // Fallback: text input with schema description as placeholder
    let placeholder = schema
        .description
        .as_ref()
        .map(std::string::ToString::to_string);
    (
        ElicitationSchema::Text {
            placeholder,
            max_length: None,
        },
        prop_name,
    )
}

/// Wrap a single response value into the object format rmcp expects.
///
/// If the value is already an object, it's returned as-is. Otherwise, it's wrapped
/// using the original property name from the schema.
fn wrap_response_value(value: Value, prop_name: Option<&str>) -> Value {
    if value.is_object() {
        // Already an object, assume it matches the expected schema
        value
    } else if let Some(name) = prop_name {
        let mut map = serde_json::Map::new();
        map.insert(name.to_string(), value);
        Value::Object(map)
    } else {
        value
    }
}

// ─── rmcp ClientHandler bridge ───────────────────────────────────────────────

use rmcp::model::{
    ClientCapabilities, ClientInfo, CreateMessageRequestParams, CreateMessageResult,
    ElicitationCapability, FormElicitationCapability, Implementation, RootsCapabilities,
    SamplingCapability, SamplingMessageContent, UrlElicitationCapability,
};
use rmcp::model::{CreateElicitationResult, ElicitationAction as RmcpElicitationAction};
use rmcp::model::{ListRootsResult, Role};
use rmcp::service::{NotificationContext, RequestContext, RoleClient};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use astrid_core::{
    ConnectorCapabilities, ConnectorDescriptor, ConnectorId, ConnectorProfile, ConnectorSource,
    FrontendType, InboundMessage, MAX_CONNECTORS_PER_PLUGIN,
};

use crate::types::ToolDefinition;

/// Maximum channels a single plugin can register (bounds memory from untrusted plugins).
const MAX_CHANNELS_PER_PLUGIN: usize = MAX_CONNECTORS_PER_PLUGIN;
/// Maximum length of a channel name in bytes.
const MAX_CHANNEL_NAME_LEN: usize = 128;

/// Channel info as sent by the bridge's `connectorRegistered` notification.
///
/// # Trust boundary
///
/// This struct is deserialized from untrusted plugin subprocess output.
/// All fields must be validated before use. The [`definition`](Self::definition)
/// field is typed (not arbitrary JSON) to bound memory usage.
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeChannelInfo {
    /// Channel name (e.g. "telegram", "discord").
    pub name: String,
    /// Optional channel definition metadata from the plugin.
    /// Typed to bound memory — only known fields are retained.
    #[serde(default)]
    pub definition: Option<BridgeChannelDefinition>,
}

/// Typed subset of the bridge channel definition.
///
/// Only the fields we actually use are retained; unknown fields are
/// silently discarded by serde, preventing unbounded memory allocation
/// from a malicious plugin.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BridgeChannelDefinition {
    /// Capability hints declared by the plugin for this channel.
    #[serde(default)]
    pub capabilities: Option<BridgeChannelCapabilities>,
}

/// Capability flags as declared by the bridge plugin (camelCase JSON).
///
/// Maps to [`ConnectorCapabilities`](astrid_core::connector::ConnectorCapabilities)
/// but uses camelCase field names matching the bridge's JSON format.
/// All flags default to `false` (least privilege).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct BridgeChannelCapabilities {
    /// Can receive inbound messages from users.
    #[serde(default)]
    pub can_receive: bool,
    /// Can send outbound messages to users.
    #[serde(default)]
    pub can_send: bool,
    /// Can present approval requests to a human.
    #[serde(default)]
    pub can_approve: bool,
    /// Can present elicitation requests to a human.
    #[serde(default)]
    pub can_elicit: bool,
    /// Supports rich media (images, embeds, etc.).
    #[serde(default)]
    pub supports_rich_media: bool,
    /// Supports threaded conversations.
    #[serde(default)]
    pub supports_threads: bool,
    /// Supports interactive buttons / action rows.
    #[serde(default)]
    pub supports_buttons: bool,
}

/// Params wrapper for the `connectorRegistered` notification.
#[derive(Debug, Deserialize)]
struct ConnectorRegisteredParams {
    #[serde(rename = "pluginId")]
    plugin_id: String,
    channels: Vec<BridgeChannelInfo>,
}

/// Returns `true` if the channel name contains only safe characters.
///
/// Allowed: ASCII alphanumeric, hyphens, underscores. Must be non-empty.
/// This prevents path traversal, null bytes, shell metacharacters, and
/// Unicode lookalikes from reaching downstream code.
fn is_valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Notification from a running MCP server about a state change.
///
/// Sent over an internal channel from `AstridClientHandler` to `McpClient`
/// so that tools caches and other state can be updated without polling.
///
/// # Trust boundary
///
/// The [`ConnectorsRegistered`](Self::ConnectorsRegistered) variant carries
/// data deserialized from an untrusted plugin subprocess. Consumers must
/// validate channel names, capabilities, and counts before using the data
/// for access-control decisions.
pub enum ServerNotice {
    /// The server pushed `notifications/tools/list_changed`; the handler has
    /// already re-fetched the tool list and attached it here.
    ToolsRefreshed {
        /// Name of the server whose tools changed.
        server_name: String,
        /// Updated tool list (already converted to `ToolDefinition`).
        tools: Vec<ToolDefinition>,
    },
    /// The bridge sent `notifications/astrid.connectorRegistered` with a batch
    /// of channel registrations after the MCP handshake completed.
    ConnectorsRegistered {
        /// Name of the MCP server (e.g. `"plugin:my-plugin"`).
        server_name: String,
        /// Channels registered by the plugin.
        channels: Vec<BridgeChannelInfo>,
    },
}

/// Maximum payload size for custom notifications (1 MB).
///
/// Checked against re-serialized JSON, which may differ from wire size
/// (e.g. due to Unicode escape compression). This is a best-effort
/// post-parse heuristic; the true wire-level bound would require
/// transport-layer enforcement.
const MAX_NOTIFICATION_PAYLOAD_BYTES: usize = 1_024 * 1_024;

/// Maximum length for a platform user ID (512 bytes, truncated at
/// a valid UTF-8 character boundary).
const MAX_PLATFORM_USER_ID_BYTES: usize = 512;

/// Maximum size for the opaque context JSON payload in inbound messages (64 KB).
const MAX_CONTEXT_BYTES: usize = 64 * 1024;

/// Bridge between astrid capability handlers and the rmcp `ClientHandler` trait.
///
/// This is the handler passed to `rmcp::ServiceExt::serve()` when connecting
/// to an MCP server. It delegates server-initiated requests (sampling, roots,
/// elicitation) to the configured `CapabilitiesHandler`.
pub struct AstridClientHandler {
    server_name: String,
    inner: Arc<CapabilitiesHandler>,
    /// Channel for pushing notifications (tools changed, etc.) back to the
    /// `McpClient`. `None` if the caller does not care about notifications.
    notice_tx: Option<mpsc::UnboundedSender<ServerNotice>>,
    /// Plugin ID for anti-spoofing validation on inbound notifications.
    plugin_id: String,
    /// Channel for inbound messages from the bridge.
    /// Bounded to 256 (set by caller in `McpPlugin::load()`).
    inbound_tx: Option<mpsc::Sender<InboundMessage>>,
    /// Shared registered connectors for connector ID lookups on inbound messages.
    ///
    /// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because this is only
    /// accessed in non-async `handle_inbound_message` and updated during
    /// connector registration. The lock is never held across an `.await` point,
    /// so a blocking mutex is correct and avoids the overhead of an async-aware mutex.
    registered_connectors: Arc<Mutex<Vec<ConnectorDescriptor>>>,
}

impl AstridClientHandler {
    /// Create a new handler for a specific server connection.
    pub fn new(server_name: impl Into<String>, inner: Arc<CapabilitiesHandler>) -> Self {
        Self {
            server_name: server_name.into(),
            inner,
            notice_tx: None,
            plugin_id: String::new(),
            inbound_tx: None,
            registered_connectors: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Attach a notice sender so that notifications (tool refreshes,
    /// connector registrations) can be forwarded to the caller.
    #[must_use]
    pub fn with_notice_tx(mut self, tx: mpsc::UnboundedSender<ServerNotice>) -> Self {
        self.notice_tx = Some(tx);
        self
    }

    /// Set the plugin ID for anti-spoofing validation on inbound notifications.
    ///
    /// **Required** when inbound message channels are configured — an empty plugin ID
    /// causes the inbound message handler to reject all messages.
    #[must_use]
    pub fn with_plugin_id(mut self, plugin_id: &str) -> Self {
        self.plugin_id = plugin_id.to_string();
        self
    }

    /// Set the channel for inbound messages from the bridge.
    #[must_use]
    pub fn with_inbound_tx(mut self, tx: mpsc::Sender<InboundMessage>) -> Self {
        self.inbound_tx = Some(tx);
        self
    }

    /// Share the registered connectors state for connector ID lookups.
    #[must_use]
    pub fn with_shared_connectors(
        mut self,
        connectors: Arc<Mutex<Vec<ConnectorDescriptor>>>,
    ) -> Self {
        self.registered_connectors = connectors;
        self
    }

    /// Handle a `notifications/astrid.inboundMessage` notification.
    fn handle_inbound_message(&self, params: Option<Value>) {
        let Some(ref tx) = self.inbound_tx else {
            debug!("Ignoring inboundMessage: no inbound_tx configured");
            return;
        };

        // Reject if no plugin_id is configured (prevents empty-string bypass)
        if self.plugin_id.is_empty() {
            warn!("inboundMessage: no plugin_id configured, rejecting");
            return;
        }

        let Some(params) = params else {
            warn!("inboundMessage: missing params");
            return;
        };

        // Validate payload size (best-effort post-parse check)
        if estimate_json_size(&params) > MAX_NOTIFICATION_PAYLOAD_BYTES {
            warn!(
                max = MAX_NOTIFICATION_PAYLOAD_BYTES,
                "inboundMessage: payload too large, rejecting"
            );
            return;
        }

        // Extract and validate plugin_id BEFORE any content allocation (anti-spoofing)
        let Some(plugin_id) = params.get("pluginId").and_then(Value::as_str) else {
            warn!("inboundMessage: missing pluginId");
            return;
        };
        if plugin_id != self.plugin_id {
            warn!(
                got = %plugin_id,
                "inboundMessage: pluginId mismatch, rejecting"
            );
            return;
        }

        let Some(content) = extract_inbound_content(&params) else {
            return;
        };

        let msg_context = params.get("context").cloned().unwrap_or(Value::Null);

        // Post-serialization context size check guards against escape-amplification
        // (control chars can expand up to 6x as \uNNNN when serialized).
        let ctx_size = if msg_context.is_null() {
            4
        } else {
            msg_context.to_string().len()
        };
        if ctx_size > MAX_CONTEXT_BYTES {
            warn!(
                max = MAX_CONTEXT_BYTES,
                actual = ctx_size,
                "inboundMessage: context payload too large, rejecting"
            );
            return;
        }

        // Extract platform_user_id with fallback chain
        let platform_user_id = extract_platform_user_id(&msg_context);

        // Extract channel name from context for connector lookup
        let channel_name = msg_context
            .get("channel")
            .or_else(|| msg_context.get("channelName"))
            .and_then(Value::as_str);

        // Resolve connector_id and platform from registered connectors
        let (connector_id, platform) = {
            let connectors = self
                .registered_connectors
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(desc) = channel_name.and_then(|ch| connectors.iter().find(|d| d.name == ch))
            {
                (desc.id, desc.frontend_type.clone())
            } else if let Some(desc) = connectors.first() {
                warn!(
                    plugin_id = %plugin_id,
                    channel = ?channel_name,
                    fallback_connector = %desc.name,
                    "inboundMessage: channel not found, falling back to first connector"
                );
                (desc.id, desc.frontend_type.clone())
            } else {
                warn!(
                    plugin_id = %plugin_id,
                    channel = ?channel_name,
                    "inboundMessage: no connectors registered, using ephemeral ID"
                );
                (
                    ConnectorId::new(),
                    FrontendType::Custom(plugin_id.to_string()),
                )
            }
        };

        // Build inbound message
        let message = InboundMessage::builder(connector_id, platform, platform_user_id, content)
            .context(msg_context)
            .build();

        // Send via bounded channel
        if let Err(e) = tx.try_send(message) {
            warn!(
                error = %e,
                "inboundMessage: inbound channel full or closed, dropping"
            );
        }
    }

    /// Process validated channels from a `connectorRegistered` notification
    /// and populate `registered_connectors` for inbound message lookups.
    fn register_channels_locally(&self, plugin_id: &str, channels: &[BridgeChannelInfo]) {
        let source = match ConnectorSource::new_openclaw(plugin_id) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "register_channels_locally: invalid plugin_id for ConnectorSource");
                return;
            },
        };

        let mut shared = self
            .registered_connectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        for ch in channels {
            // Skip duplicates
            if shared.iter().any(|d| d.name == ch.name) {
                continue;
            }

            // Map platform from channel name (best effort)
            let frontend_type = map_platform_name(&ch.name);

            // Map capabilities from typed definition
            let capabilities = ch
                .definition
                .as_ref()
                .and_then(|d| d.capabilities.as_ref())
                .map_or_else(ConnectorCapabilities::receive_only, |caps| {
                    ConnectorCapabilities {
                        can_receive: caps.can_receive,
                        can_send: caps.can_send,
                        can_approve: caps.can_approve,
                        ..ConnectorCapabilities::default()
                    }
                });

            let descriptor = ConnectorDescriptor::builder(&ch.name, frontend_type)
                .source(source.clone())
                .profile(ConnectorProfile::Bridge)
                .capabilities(capabilities)
                .build();

            shared.push(descriptor);
        }
    }
}

impl rmcp::ClientHandler for AstridClientHandler {
    fn get_info(&self) -> ClientInfo {
        // Build capabilities directly to avoid typestate builder limitations
        // with conditional enable_* calls.
        let capabilities = ClientCapabilities {
            roots: if self.inner.has_roots() {
                Some(RootsCapabilities::default())
            } else {
                None
            },
            sampling: if self.inner.has_sampling() {
                Some(SamplingCapability::default())
            } else {
                None
            },
            elicitation: {
                let has_form = self.inner.has_elicitation();
                let has_url = self.inner.has_url_elicitation();
                if has_form || has_url {
                    Some(ElicitationCapability {
                        form: has_form.then(FormElicitationCapability::default),
                        url: has_url.then(UrlElicitationCapability::default),
                    })
                } else {
                    None
                }
            },
            ..Default::default()
        };

        ClientInfo {
            meta: None,
            protocol_version: serde_json::from_value(serde_json::json!("2025-11-25"))
                .expect("valid protocol version"),
            capabilities,
            client_info: Implementation {
                name: "astrid".to_string(),
                title: Some("Astrid Secure Agent Runtime".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
        }
    }

    async fn create_message(
        &self,
        params: CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateMessageResult, rmcp::ErrorData> {
        let Some(ref sampling) = self.inner.sampling else {
            return Err(rmcp::ErrorData::internal_error(
                "Sampling not supported",
                None,
            ));
        };

        // Convert rmcp SamplingMessages to astrid SamplingMessages.
        // Each rmcp message may carry Single or Multiple content items;
        // we take the first supported item for our simpler representation.
        let messages = params
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                // Find the first supported content item (Text/Image) from Single/Multiple.
                let first_supported = match &m.content {
                    rmcp::model::SamplingContent::Single(item) => Some(item),
                    rmcp::model::SamplingContent::Multiple(items) => items.iter().find(|item| {
                        matches!(
                            item,
                            SamplingMessageContent::Text(_) | SamplingMessageContent::Image(_)
                        )
                    }),
                };
                let content = match first_supported {
                    Some(SamplingMessageContent::Text(t)) => SamplingContent::Text {
                        text: t.text.clone(),
                    },
                    Some(SamplingMessageContent::Image(i)) => SamplingContent::Image {
                        data: i.data.clone(),
                        mime_type: i.mime_type.clone(),
                    },
                    _ => SamplingContent::Text {
                        text: "[unsupported content type]".to_string(),
                    },
                };
                SamplingMessage {
                    role: role.to_string(),
                    content,
                }
            })
            .collect();

        let request = SamplingRequest {
            request_id: Uuid::new_v4(),
            server: self.server_name.clone(),
            messages,
            system: params.system_prompt,
            max_tokens: Some(params.max_tokens),
            temperature: params.temperature.map(f64::from),
            model_hint: None,
            metadata: HashMap::new(),
        };

        let response = sampling.handle_sampling(request).await;

        if !response.success {
            return Err(rmcp::ErrorData::internal_error(
                response.error.unwrap_or_default(),
                None,
            ));
        }

        let text = response.content.unwrap_or_default();
        Ok(CreateMessageResult {
            model: response.model.unwrap_or_else(|| "unknown".to_string()),
            stop_reason: response.stop_reason,
            message: rmcp::model::SamplingMessage::assistant_text(text),
        })
    }

    async fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, rmcp::ErrorData> {
        let Some(ref roots_handler) = self.inner.roots else {
            return Err(rmcp::ErrorData::internal_error("Roots not supported", None));
        };

        let request = RootsRequest {
            request_id: Uuid::new_v4(),
            server: self.server_name.clone(),
        };

        let response = roots_handler.handle_roots(request).await;

        Ok(ListRootsResult {
            roots: response
                .roots
                .into_iter()
                .map(|r| rmcp::model::Root {
                    uri: r.uri,
                    name: r.name,
                })
                .collect(),
        })
    }

    async fn create_elicitation(
        &self,
        request: CreateElicitationRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, rmcp::ErrorData> {
        match request {
            CreateElicitationRequestParams::FormElicitationParams {
                message,
                requested_schema,
                ..
            } => {
                let Some(ref handler) = self.inner.elicitation else {
                    return Err(rmcp::ErrorData::internal_error(
                        "Elicitation not supported",
                        None,
                    ));
                };

                // Convert rmcp schema to core schema
                let (core_schema, prop_name) = convert_rmcp_schema(&requested_schema);

                // Determine if the elicited property is required based on the schema's required list
                let required = match (requested_schema.required.as_ref(), prop_name.as_deref()) {
                    (Some(required_fields), Some(name)) => {
                        required_fields.iter().any(|field| field == name)
                    },
                    _ => false,
                };

                let core_request =
                    ElicitationRequest::new(&self.server_name, &message).with_schema(core_schema);
                let core_request = if required {
                    core_request
                } else {
                    core_request.optional()
                };

                let response = handler.handle_elicitation(core_request).await;

                // Convert core response to rmcp result
                match response.action {
                    CoreElicitationAction::Submit { value } => {
                        let content = wrap_response_value(value, prop_name.as_deref());
                        Ok(CreateElicitationResult {
                            action: RmcpElicitationAction::Accept,
                            content: Some(content),
                        })
                    },
                    CoreElicitationAction::Cancel => Ok(CreateElicitationResult {
                        action: RmcpElicitationAction::Cancel,
                        content: None,
                    }),
                    CoreElicitationAction::Dismiss => Ok(CreateElicitationResult {
                        action: RmcpElicitationAction::Decline,
                        content: None,
                    }),
                }
            },
            CreateElicitationRequestParams::UrlElicitationParams { message, url, .. } => {
                let Some(ref handler) = self.inner.url_elicitation else {
                    return Err(rmcp::ErrorData::internal_error(
                        "URL elicitation not supported",
                        None,
                    ));
                };

                let core_request = UrlElicitationRequest::new(&self.server_name, &url, &message);
                let response = handler.handle_url_elicitation(core_request).await;

                if response.completed {
                    Ok(CreateElicitationResult {
                        action: RmcpElicitationAction::Accept,
                        content: None,
                    })
                } else {
                    Ok(CreateElicitationResult {
                        action: RmcpElicitationAction::Decline,
                        content: None,
                    })
                }
            },
        }
    }

    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        let server = &self.server_name;
        info!(server = %server, "Received tools/list_changed notification");

        // Re-fetch the full tool list from the server.
        let tools = match context.peer.list_all_tools().await {
            Ok(rmcp_tools) => rmcp_tools
                .iter()
                .map(|t| ToolDefinition::from_rmcp(t, server))
                .collect::<Vec<_>>(),
            Err(e) => {
                warn!(
                    server = %server,
                    error = %e,
                    "Failed to re-fetch tools after list_changed notification"
                );
                return;
            },
        };

        info!(
            server = %server,
            tool_count = tools.len(),
            "Refreshed tool list after notification"
        );

        // Push the refreshed list to the McpClient via the notice channel.
        if let Some(ref tx) = self.notice_tx {
            let _ = tx.send(ServerNotice::ToolsRefreshed {
                server_name: server.clone(),
                tools,
            });
        }
    }

    async fn on_custom_notification(
        &self,
        notification: rmcp::model::CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) {
        match notification.method.as_str() {
            "notifications/astrid.connectorRegistered" => {
                if let Some(ref tx) = self.notice_tx {
                    match notification.params_as::<ConnectorRegisteredParams>() {
                        Ok(Some(params)) => {
                            // Log if the plugin claims a different identity than expected.
                            // server_name is "plugin:<id>"; strip the prefix for exact match.
                            let expected_id = self
                                .server_name
                                .strip_prefix("plugin:")
                                .unwrap_or(&self.server_name);
                            if params.plugin_id != expected_id {
                                warn!(
                                    server = %self.server_name,
                                    claimed_id = %params.plugin_id,
                                    "connectorRegistered: pluginId mismatch"
                                );
                            }
                            // Validate: cap channels, enforce name length + character set.
                            let channels: Vec<_> = params
                                .channels
                                .into_iter()
                                .take(MAX_CHANNELS_PER_PLUGIN)
                                .filter(|ch| {
                                    ch.name.len() <= MAX_CHANNEL_NAME_LEN
                                        && is_valid_channel_name(&ch.name)
                                })
                                .collect();
                            if channels.is_empty() {
                                return;
                            }
                            // Also register locally for inbound message connector lookups
                            self.register_channels_locally(expected_id, &channels);
                            let _ = tx.send(ServerNotice::ConnectorsRegistered {
                                server_name: self.server_name.clone(),
                                channels,
                            });
                        },
                        Ok(None) => {
                            warn!(
                                server = %self.server_name,
                                "connectorRegistered: missing params"
                            );
                        },
                        Err(e) => {
                            warn!(
                                server = %self.server_name,
                                error = %e,
                                "connectorRegistered: failed to parse params"
                            );
                        },
                    }
                }
            },
            "notifications/astrid.inboundMessage" => {
                self.handle_inbound_message(notification.params);
            },
            // Note: `notifications/astrid.configChanged` is sent by the bridge
            // when plugin config is written. Currently informational only; a
            // future phase may reload config or forward to the runtime.
            other => {
                debug!(
                    server = %self.server_name,
                    method = %other,
                    "Ignoring unknown custom notification"
                );
            },
        }
    }
}

/// Maximum length for a custom platform name (128 bytes).
///
/// Platform names that exceed this limit are truncated at a UTF-8 character
/// boundary. Known platform names (discord, telegram, etc.) are never
/// affected since they are matched before the custom fallback.
const MAX_PLATFORM_NAME_BYTES: usize = 128;

/// Map a platform name string to a [`FrontendType`].
///
/// Custom platform names are truncated to [`MAX_PLATFORM_NAME_BYTES`].
fn map_platform_name(name: &str) -> FrontendType {
    match name.to_lowercase().as_str() {
        "telegram" => FrontendType::Telegram,
        "discord" => FrontendType::Discord,
        "slack" => FrontendType::Slack,
        "whatsapp" => FrontendType::WhatsApp,
        "web" => FrontendType::Web,
        "cli" => FrontendType::Cli,
        other => {
            let truncated = if other.len() > MAX_PLATFORM_NAME_BYTES {
                &other[..other.floor_char_boundary(MAX_PLATFORM_NAME_BYTES)]
            } else {
                other
            };
            FrontendType::Custom(truncated.to_string())
        },
    }
}

/// Parse connector capabilities from a channel definition JSON object.
///
/// Looks for `chatTypes` or `capabilities` arrays in the definition and maps
/// known strings to capability flags. Falls back to `receive_only()`.
///
/// Recognized strings (case-insensitive):
/// - `"receive"`, `"inbound"` → `can_receive`
/// - `"send"`, `"outbound"` → `can_send`
/// - `"chat"` → both `can_receive` and `can_send` (bidirectional)
/// - `"approve"` → `can_approve`
///
/// Other strings and non-string array elements are silently ignored.
/// Fields like `can_elicit`, `supports_rich_media`, `supports_threads`,
/// and `supports_buttons` are not yet parsed from the bridge definition
/// and default to `false`.
#[cfg(test)]
fn parse_connector_capabilities(definition: &Value) -> ConnectorCapabilities {
    let caps_array = definition
        .get("capabilities")
        .or_else(|| definition.get("chatTypes"))
        .and_then(Value::as_array);

    let Some(arr) = caps_array else {
        return ConnectorCapabilities::receive_only();
    };

    let lowered: Vec<String> = arr
        .iter()
        .take(64) // cap allocation against adversarial arrays
        .filter_map(Value::as_str)
        .map(str::to_lowercase)
        .collect();
    if lowered.is_empty() {
        return ConnectorCapabilities::receive_only();
    }

    let can_receive = lowered
        .iter()
        .any(|s| s == "receive" || s == "inbound" || s == "chat");
    let can_send = lowered
        .iter()
        .any(|s| s == "send" || s == "outbound" || s == "chat");
    let can_approve = lowered.iter().any(|s| s == "approve");

    // If we parsed something meaningful, build from flags; otherwise receive_only
    if can_receive || can_send || can_approve {
        ConnectorCapabilities {
            can_receive,
            can_send,
            can_approve,
            ..ConnectorCapabilities::default()
        }
    } else {
        ConnectorCapabilities::receive_only()
    }
}

/// Extract `platform_user_id` from an inbound message context JSON, with
/// fallback chain: `context.from.id` → `context.senderId` → `context.userId`
/// → `"unknown"`. Truncated to [`MAX_PLATFORM_USER_ID_BYTES`] at a valid
/// UTF-8 character boundary.
fn extract_platform_user_id(context: &Value) -> String {
    let raw = context
        .get("from")
        .and_then(|f| f.get("id"))
        .and_then(Value::as_str)
        .or_else(|| context.get("senderId").and_then(Value::as_str))
        .or_else(|| context.get("userId").and_then(Value::as_str))
        .unwrap_or("unknown");

    if raw.len() > MAX_PLATFORM_USER_ID_BYTES {
        // Truncate at a valid UTF-8 character boundary to avoid panics
        // on multi-byte characters that straddle the limit.
        let boundary = raw.floor_char_boundary(MAX_PLATFORM_USER_ID_BYTES);
        raw[..boundary].to_string()
    } else {
        raw.to_string()
    }
}

/// Extract and validate the `content` field from inbound message params.
///
/// Returns `None` (with a warning) if content is missing, null, empty, or
/// exceeds [`MAX_NOTIFICATION_PAYLOAD_BYTES`]. Non-string values are
/// serialized to JSON with a post-serialization size check to guard against
/// escape-amplification attacks.
fn extract_inbound_content(params: &Value) -> Option<String> {
    let Some(content_val) = params.get("content").filter(|v| !v.is_null()) else {
        warn!("inboundMessage: missing or null content");
        return None;
    };
    if let Some(s) = content_val.as_str() {
        if s.is_empty() || s.len() > MAX_NOTIFICATION_PAYLOAD_BYTES {
            warn!("inboundMessage: string content empty or exceeds size limit");
            return None;
        }
        Some(s.to_string())
    } else {
        // Non-string content (objects, arrays) is serialized. Check the
        // expanded size to guard against escape-amplification attacks
        // (e.g. control chars expanding 1 byte → 6 bytes as \uNNNN).
        let serialized = content_val.to_string();
        if serialized.len() > MAX_NOTIFICATION_PAYLOAD_BYTES {
            warn!("inboundMessage: serialized content exceeds limit after expansion");
            return None;
        }
        Some(serialized)
    }
}

/// Estimate the serialized JSON size of a [`Value`] by walking the parsed tree.
///
/// Counts bytes for keys, string values, structural characters, and numeric
/// representations. **Known limitation:** strings containing JSON-escaped
/// characters (control chars `\x00`–`\x1f`, backslashes, quotes) can expand
/// up to 6× when re-serialized (e.g. `\x00` → `\u0000`). This means the
/// estimate may *undercount* by up to 6× in adversarial payloads.
///
/// # Recursion safety
///
/// This function recurses into nested arrays and objects. Its stack depth is
/// bounded by `serde_json`'s default recursion limit (128 levels), which is
/// applied during parsing.
fn estimate_json_size(value: &Value) -> usize {
    match value {
        Value::Null => 4, // "null"
        Value::Bool(b) => {
            if *b { 4 } else { 5 } // "true" / "false"
        },
        Value::Number(n) => {
            // Allocates briefly for digit count; accurate for small numbers.
            n.to_string().len()
        },
        Value::String(s) => {
            // 2 for quotes + string length (ignoring escape expansion)
            s.len().saturating_add(2)
        },
        Value::Array(arr) => {
            // 2 for [] + commas + recursive sizes
            let inner: usize = arr.iter().map(estimate_json_size).sum();
            let commas = arr.len().saturating_sub(1);
            inner.saturating_add(2).saturating_add(commas)
        },
        Value::Object(map) => {
            // 2 for {} + commas + key/value pairs
            let inner: usize = map
                .iter()
                .map(|(k, v)| {
                    // key: 2 quotes + len + colon + value
                    k.len()
                        .saturating_add(3)
                        .saturating_add(estimate_json_size(v))
                })
                .sum();
            let commas = map.len().saturating_sub(1);
            inner.saturating_add(2).saturating_add(commas)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sampling_request_serialization() {
        let request = SamplingRequest {
            request_id: Uuid::new_v4(),
            server: "test".to_string(),
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text {
                    text: "Hello".to_string(),
                },
            }],
            system: None,
            max_tokens: Some(100),
            temperature: Some(0.7),
            model_hint: None,
            metadata: HashMap::new(),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"server\":\"test\""));
        assert!(json.contains("\"max_tokens\":100"));
    }

    #[test]
    fn test_convert_rmcp_schema_boolean() {
        let rmcp_schema: rmcp::model::ElicitationSchema =
            serde_json::from_value(serde_json::json!({
                "type": "object",
                "properties": {
                    "confirmed": {
                        "type": "boolean",
                        "description": "Confirm the action"
                    }
                }
            }))
            .unwrap();

        let (schema, prop_name) = convert_rmcp_schema(&rmcp_schema);
        assert!(matches!(
            schema,
            ElicitationSchema::Confirm { default: false }
        ));
        assert_eq!(prop_name, Some("confirmed".to_string()));
    }

    #[test]
    fn test_convert_rmcp_schema_string() {
        let rmcp_schema: rmcp::model::ElicitationSchema =
            serde_json::from_value(serde_json::json!({
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "description": "Enter your API key",
                        "maxLength": 128
                    }
                }
            }))
            .unwrap();

        let (schema, prop_name) = convert_rmcp_schema(&rmcp_schema);
        assert!(matches!(
            schema,
            ElicitationSchema::Text {
                placeholder: Some(_),
                max_length: Some(128),
            }
        ));
        assert_eq!(prop_name, Some("api_key".to_string()));
    }

    #[test]
    fn test_wrap_response_value_primitive() {
        let value = Value::String("hello".to_string());
        let wrapped = wrap_response_value(value, Some("key"));
        assert_eq!(wrapped, serde_json::json!({"key": "hello"}));
    }

    #[test]
    fn test_wrap_response_value_object_passthrough() {
        let obj = serde_json::json!({"a": 1, "b": 2});
        let passthrough = wrap_response_value(obj.clone(), Some("key"));
        assert_eq!(passthrough, obj);
    }

    #[test]
    fn test_wrap_response_value_no_prop_name() {
        let value = Value::String("hello".to_string());
        let result = wrap_response_value(value.clone(), None);
        assert_eq!(result, value);
    }

    #[test]
    fn test_capabilities_handler_builder() {
        let handler = CapabilitiesHandler::new();
        assert!(!handler.has_sampling());
        assert!(!handler.has_roots());
        assert!(!handler.has_elicitation());
        assert!(!handler.has_url_elicitation());
    }

    #[test]
    fn test_connector_registered_params_deserialization() {
        let json = serde_json::json!({
            "pluginId": "channel-echo",
            "channels": [{
                "name": "telegram",
                "definition": {
                    "description": "Telegram connector",
                    "capabilities": { "canReceive": true, "canSend": true }
                }
            }]
        });

        let params: ConnectorRegisteredParams =
            serde_json::from_value(json).expect("should parse ConnectorRegisteredParams");
        assert_eq!(params.plugin_id, "channel-echo");
        assert_eq!(params.channels.len(), 1);
        assert_eq!(params.channels[0].name, "telegram");

        let def = params.channels[0].definition.as_ref().expect("definition");
        let caps = def.capabilities.as_ref().expect("capabilities");
        assert!(caps.can_receive);
        assert!(caps.can_send);
        assert!(!caps.can_approve);
    }

    #[test]
    fn test_channel_name_validation() {
        assert!(is_valid_channel_name("telegram"));
        assert!(is_valid_channel_name("my-channel"));
        assert!(is_valid_channel_name("channel_2"));
        assert!(!is_valid_channel_name(""));
        assert!(!is_valid_channel_name("../etc/passwd"));
        assert!(!is_valid_channel_name("name\0hidden"));
        assert!(!is_valid_channel_name("has spaces"));
        assert!(!is_valid_channel_name("has\nnewline"));
    }

    /// Helper: build a handler wired to inbound channel + shared connectors.
    fn test_handler(
        plugin_id: &str,
    ) -> (
        AstridClientHandler,
        mpsc::Receiver<InboundMessage>,
        Arc<Mutex<Vec<ConnectorDescriptor>>>,
    ) {
        let (inbound_tx, inbound_rx) = mpsc::channel(256);
        let shared = Arc::new(Mutex::new(Vec::new()));
        let handler = AstridClientHandler::new("test-server", Arc::new(CapabilitiesHandler::new()))
            .with_plugin_id(plugin_id)
            .with_inbound_tx(inbound_tx)
            .with_shared_connectors(Arc::clone(&shared));
        (handler, inbound_rx, shared)
    }

    /// Helper: register a connector in `shared_connectors` for inbound message
    /// tests. This simulates what `register_channels_locally` does during
    /// `on_custom_notification`.
    fn register_test_connector(
        shared: &Arc<Mutex<Vec<ConnectorDescriptor>>>,
        name: &str,
        platform: FrontendType,
        plugin_id: &str,
    ) -> ConnectorId {
        let source = ConnectorSource::new_openclaw(plugin_id).expect("valid plugin_id");
        let descriptor = ConnectorDescriptor::builder(name, platform)
            .source(source)
            .profile(ConnectorProfile::Bridge)
            .capabilities(ConnectorCapabilities::receive_only())
            .build();
        let id = descriptor.id;
        shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(descriptor);
        id
    }

    #[test]
    fn test_inbound_message_notification() {
        let (handler, mut inbound_rx, shared) = test_handler("test-plugin");

        // Register a connector in shared state
        let expected_id =
            register_test_connector(&shared, "telegram", FrontendType::Telegram, "test-plugin");

        // Now send an inbound message
        let msg_params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "Hello from Telegram",
            "context": {
                "channel": "telegram",
                "from": { "id": "user-123" }
            }
        });
        handler.handle_inbound_message(Some(msg_params));

        let msg = inbound_rx.try_recv().expect("should receive message");
        assert_eq!(msg.connector_id, expected_id);
        assert!(matches!(msg.platform, FrontendType::Telegram));
        assert_eq!(msg.platform_user_id, "user-123");
        assert_eq!(msg.content, "Hello from Telegram");
    }

    #[test]
    fn test_inbound_message_oversized_rejected() {
        let (handler, mut rx, _) = test_handler("test-plugin");

        // Build a payload > 1 MB
        let big_content = "x".repeat(MAX_NOTIFICATION_PAYLOAD_BYTES + 100);
        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": big_content,
            "context": {}
        });

        handler.handle_inbound_message(Some(params));
        assert!(
            rx.try_recv().is_err(),
            "oversized message should be rejected"
        );
    }

    #[test]
    fn test_inbound_message_full_channel_drops() {
        // Create a handler with a channel of size 1
        let (inbound_tx, mut inbound_rx) = mpsc::channel(1);
        let handler = AstridClientHandler::new("test-server", Arc::new(CapabilitiesHandler::new()))
            .with_plugin_id("test-plugin")
            .with_inbound_tx(inbound_tx);

        let make_params = || {
            serde_json::json!({
                "pluginId": "test-plugin",
                "content": "msg",
                "context": {}
            })
        };

        // Fill the single-slot buffer
        handler.handle_inbound_message(Some(make_params()));

        // Channel is now full — this message should be dropped via try_send
        handler.handle_inbound_message(Some(make_params()));

        // Only one message should be in the buffer
        assert!(
            inbound_rx.try_recv().is_ok(),
            "first message should be present"
        );
        assert!(
            inbound_rx.try_recv().is_err(),
            "second message should have been dropped"
        );
    }

    #[test]
    fn test_inbound_message_plugin_id_mismatch() {
        let (handler, mut rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "evil-plugin",
            "content": "hijack",
            "context": {}
        });

        handler.handle_inbound_message(Some(params));
        assert!(
            rx.try_recv().is_err(),
            "mismatched plugin_id should be rejected"
        );
    }

    #[test]
    fn test_map_platform_name() {
        assert!(matches!(
            map_platform_name("Telegram"),
            FrontendType::Telegram
        ));
        assert!(matches!(
            map_platform_name("DISCORD"),
            FrontendType::Discord
        ));
        assert!(matches!(map_platform_name("slack"), FrontendType::Slack));
        assert!(matches!(
            map_platform_name("WhatsApp"),
            FrontendType::WhatsApp
        ));
        assert!(matches!(map_platform_name("web"), FrontendType::Web));
        assert!(matches!(map_platform_name("cli"), FrontendType::Cli));
        assert!(matches!(
            map_platform_name("matrix"),
            FrontendType::Custom(_)
        ));
        if let FrontendType::Custom(name) = map_platform_name("Matrix") {
            assert_eq!(name, "matrix");
        }
    }

    #[test]
    fn test_parse_connector_capabilities_chat() {
        let def = serde_json::json!({ "capabilities": ["receive", "send", "approve"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
        assert!(caps.can_approve);
    }

    #[test]
    fn test_parse_connector_capabilities_fallback() {
        let def = serde_json::json!({});
        let caps = parse_connector_capabilities(&def);
        assert_eq!(caps, ConnectorCapabilities::receive_only());
    }

    #[test]
    fn test_parse_connector_capabilities_chat_types_key() {
        let def = serde_json::json!({ "chatTypes": ["receive", "send"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
        assert!(!caps.can_approve);
    }

    #[test]
    fn test_parse_connector_capabilities_chat_bidirectional() {
        let def = serde_json::json!({ "capabilities": ["chat"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
    }

    #[test]
    fn test_extract_platform_user_id_from_id() {
        let ctx = serde_json::json!({ "from": { "id": "user-42" } });
        assert_eq!(extract_platform_user_id(&ctx), "user-42");
    }

    #[test]
    fn test_extract_platform_user_id_sender_id() {
        let ctx = serde_json::json!({ "senderId": "sender-99" });
        assert_eq!(extract_platform_user_id(&ctx), "sender-99");
    }

    #[test]
    fn test_extract_platform_user_id_fallback() {
        let ctx = serde_json::json!({});
        assert_eq!(extract_platform_user_id(&ctx), "unknown");
    }

    #[test]
    fn test_extract_platform_user_id_truncated() {
        let long_id = "x".repeat(MAX_PLATFORM_USER_ID_BYTES + 100);
        let ctx = serde_json::json!({ "senderId": long_id });
        let result = extract_platform_user_id(&ctx);
        assert_eq!(result.len(), MAX_PLATFORM_USER_ID_BYTES);
    }

    #[test]
    fn test_extract_platform_user_id_multibyte_truncation() {
        let emoji = "\u{1F600}"; // 4 bytes each
        let count = MAX_PLATFORM_USER_ID_BYTES / emoji.len() + 5;
        let long_id: String = emoji.repeat(count);
        assert!(long_id.len() > MAX_PLATFORM_USER_ID_BYTES);

        let ctx = serde_json::json!({ "senderId": long_id });
        let result = extract_platform_user_id(&ctx);
        assert!(result.len() <= MAX_PLATFORM_USER_ID_BYTES);
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_inbound_message_no_connectors_fallback() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "orphan message",
            "context": { "from": { "id": "user-1" } }
        });
        handler.handle_inbound_message(Some(params));

        let msg = inbound_rx.try_recv().expect("should receive message");
        assert_eq!(msg.content, "orphan message");
        assert!(matches!(msg.platform, FrontendType::Custom(_)));
    }

    #[test]
    fn test_inbound_message_non_matching_channel() {
        let (handler, mut inbound_rx, shared) = test_handler("test-plugin");

        register_test_connector(&shared, "telegram", FrontendType::Telegram, "test-plugin");

        let msg_params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "wrong channel",
            "context": { "channel": "discord" }
        });
        handler.handle_inbound_message(Some(msg_params));

        let msg = inbound_rx.try_recv().expect("should receive message");
        assert!(matches!(msg.platform, FrontendType::Telegram));
    }

    #[test]
    fn test_inbound_message_empty_plugin_id_rejected() {
        let (inbound_tx, mut inbound_rx) = mpsc::channel(256);
        let handler = AstridClientHandler::new("test-server", Arc::new(CapabilitiesHandler::new()))
            .with_plugin_id("")
            .with_inbound_tx(inbound_tx);

        let params = serde_json::json!({
            "pluginId": "",
            "content": "sneaky",
            "context": {}
        });
        handler.handle_inbound_message(Some(params));
        assert!(
            inbound_rx.try_recv().is_err(),
            "empty plugin_id should be rejected"
        );
    }

    #[test]
    fn test_inbound_message_non_string_content() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": { "type": "image", "url": "https://example.com/pic.png" },
            "context": {}
        });
        handler.handle_inbound_message(Some(params));

        let msg = inbound_rx.try_recv().expect("should receive message");
        let parsed: serde_json::Value =
            serde_json::from_str(&msg.content).expect("content should be valid JSON");
        assert_eq!(parsed["type"], "image");
        assert_eq!(parsed["url"], "https://example.com/pic.png");
    }

    #[test]
    fn test_inbound_message_oversized_context_rejected() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let big_context = "x".repeat(MAX_CONTEXT_BYTES + 100);
        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "msg",
            "context": { "data": big_context }
        });
        handler.handle_inbound_message(Some(params));
        assert!(
            inbound_rx.try_recv().is_err(),
            "oversized context should be rejected"
        );
    }

    #[test]
    fn test_estimate_json_size_primitives() {
        assert_eq!(estimate_json_size(&Value::Null), 4);
        assert_eq!(estimate_json_size(&Value::Bool(true)), 4);
        assert_eq!(estimate_json_size(&Value::Bool(false)), 5);
    }

    #[test]
    fn test_estimate_json_size_string() {
        let val = Value::String("hello".to_string());
        assert_eq!(estimate_json_size(&val), 7);
    }

    #[test]
    fn test_estimate_json_size_object() {
        let val = serde_json::json!({"a": 1});
        let size = estimate_json_size(&val);
        let actual = serde_json::to_string(&val).unwrap().len();
        assert!(size > 0);
        assert!(
            (size as i64 - actual as i64).unsigned_abs() <= 1,
            "estimate {size} should be within 1 of actual {actual}"
        );
    }

    #[test]
    fn test_estimate_json_size_large_payload() {
        let big = "x".repeat(MAX_NOTIFICATION_PAYLOAD_BYTES + 100);
        let val = serde_json::json!({ "data": big });
        assert!(estimate_json_size(&val) > MAX_NOTIFICATION_PAYLOAD_BYTES);
    }

    #[test]
    fn test_handlers_reject_missing_params() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");
        handler.handle_inbound_message(None);
        assert!(inbound_rx.try_recv().is_err());
    }

    #[test]
    fn test_inbound_message_channel_name_fallback_key() {
        let (handler, mut inbound_rx, shared) = test_handler("test-plugin");

        let expected_id =
            register_test_connector(&shared, "telegram", FrontendType::Telegram, "test-plugin");

        let msg_params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "via channelName",
            "context": {
                "channelName": "telegram",
                "from": { "id": "user-1" }
            }
        });
        handler.handle_inbound_message(Some(msg_params));

        let msg = inbound_rx.try_recv().expect("should receive message");
        assert_eq!(msg.connector_id, expected_id);
        assert_eq!(msg.content, "via channelName");
    }

    #[test]
    fn test_inbound_message_null_content_rejected() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": null,
            "context": {}
        });
        handler.handle_inbound_message(Some(params));

        assert!(
            inbound_rx.try_recv().is_err(),
            "null content should be rejected"
        );
    }

    #[test]
    fn test_parse_connector_capabilities_non_string_elements_ignored() {
        let def = serde_json::json!({
            "capabilities": [42, true, null, "receive", "send"]
        });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive);
        assert!(caps.can_send);
        assert!(!caps.can_approve);
    }

    #[test]
    fn test_map_platform_name_empty_string() {
        let ft = map_platform_name("");
        assert!(matches!(ft, FrontendType::Custom(ref s) if s.is_empty()));
    }

    #[test]
    fn test_handlers_accept_valid_payloads() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let msg_params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "dispatch test",
            "context": {}
        });
        handler.handle_inbound_message(Some(msg_params));
        assert!(
            inbound_rx.try_recv().is_ok(),
            "inboundMessage should dispatch"
        );
    }

    #[test]
    fn test_handlers_no_channels_configured() {
        let handler = AstridClientHandler::new("test-server", Arc::new(CapabilitiesHandler::new()));

        handler.handle_inbound_message(Some(serde_json::json!({
            "pluginId": "test",
            "content": "msg",
            "context": {}
        })));
    }

    #[test]
    fn test_parse_connector_capabilities_inbound_outbound_synonyms() {
        let def = serde_json::json!({ "capabilities": ["inbound", "outbound"] });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_receive, "inbound should set can_receive");
        assert!(caps.can_send, "outbound should set can_send");
        assert!(!caps.can_approve);
    }

    #[test]
    fn test_parse_connector_capabilities_unrecognized_strings_only() {
        let def = serde_json::json!({ "capabilities": ["foo", "bar", "baz"] });
        let caps = parse_connector_capabilities(&def);
        assert_eq!(caps, ConnectorCapabilities::receive_only());
    }

    #[test]
    fn test_parse_connector_capabilities_all_non_string_elements() {
        let def = serde_json::json!({ "capabilities": [42, true, null, [1, 2]] });
        let caps = parse_connector_capabilities(&def);
        assert_eq!(caps, ConnectorCapabilities::receive_only());
    }

    #[test]
    fn test_estimate_json_size_array() {
        let val = serde_json::json!([1, 2, 3]);
        let size = estimate_json_size(&val);
        let actual = serde_json::to_string(&val).unwrap().len();
        assert!(
            (size as i64 - actual as i64).unsigned_abs() <= 1,
            "array estimate {size} should be within 1 of actual {actual}"
        );
    }

    #[test]
    fn test_estimate_json_size_nested_array() {
        let val = serde_json::json!([[1], [2, 3]]);
        let size = estimate_json_size(&val);
        let actual = serde_json::to_string(&val).unwrap().len();
        assert!(
            (size as i64 - actual as i64).unsigned_abs() <= 2,
            "nested array estimate {size} should be within 2 of actual {actual}"
        );
    }

    #[test]
    fn test_inbound_message_empty_string_content_rejected() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "",
            "context": {}
        });
        handler.handle_inbound_message(Some(params));

        assert!(
            inbound_rx.try_recv().is_err(),
            "empty string content should be rejected"
        );
    }

    #[test]
    fn test_inbound_message_missing_plugin_id_field() {
        let (handler, mut rx, _) = test_handler("test-plugin");
        let params = serde_json::json!({
            "content": "msg",
            "context": {}
        });
        handler.handle_inbound_message(Some(params));
        assert!(
            rx.try_recv().is_err(),
            "missing pluginId field should be rejected"
        );
    }

    #[test]
    fn test_extract_platform_user_id_exactly_at_limit() {
        let id_at_limit = "x".repeat(MAX_PLATFORM_USER_ID_BYTES);
        let ctx = serde_json::json!({ "senderId": id_at_limit });
        let result = extract_platform_user_id(&ctx);
        assert_eq!(result.len(), MAX_PLATFORM_USER_ID_BYTES);
        assert_eq!(result, id_at_limit);
    }

    #[test]
    fn test_parse_connector_capabilities_capabilities_key_takes_priority() {
        let def = serde_json::json!({
            "capabilities": ["send"],
            "chatTypes": ["receive"]
        });
        let caps = parse_connector_capabilities(&def);
        assert!(caps.can_send, "capabilities key should take priority");
        assert!(
            !caps.can_receive,
            "chatTypes should be ignored when capabilities is present"
        );
    }

    #[test]
    fn test_estimate_json_size_empty_containers() {
        assert_eq!(estimate_json_size(&Value::String(String::new())), 2);
        assert_eq!(estimate_json_size(&serde_json::json!([])), 2);
        assert_eq!(estimate_json_size(&serde_json::json!({})), 2);
    }

    #[test]
    fn test_inbound_message_oversized_non_string_content_rejected() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");
        let big_array: Vec<String> = (0..50_000).map(|i| format!("item-{i:020}")).collect();
        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": big_array,
            "context": {}
        });
        handler.handle_inbound_message(Some(params));
        assert!(
            inbound_rx.try_recv().is_err(),
            "oversized serialized content should be rejected"
        );
    }

    #[test]
    fn test_inbound_message_non_string_plugin_id_rejected() {
        let (handler, mut rx, _) = test_handler("test-plugin");
        let params = serde_json::json!({
            "pluginId": 42,
            "content": "msg",
            "context": {}
        });
        handler.handle_inbound_message(Some(params));
        assert!(
            rx.try_recv().is_err(),
            "non-string pluginId should be rejected"
        );
    }

    #[test]
    fn test_extract_platform_user_id_userid_key() {
        let ctx = serde_json::json!({ "userId": "user-from-userid" });
        let id = extract_platform_user_id(&ctx);
        assert_eq!(id, "user-from-userid");
    }

    #[test]
    fn test_inbound_message_null_context() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "hello",
            "context": null
        });
        handler.handle_inbound_message(Some(params));

        let msg = inbound_rx.try_recv().expect("should handle null context");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.platform_user_id, "unknown");
    }

    #[test]
    fn test_inbound_message_absent_context() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": "hello"
        });
        handler.handle_inbound_message(Some(params));

        let msg = inbound_rx.try_recv().expect("should handle absent context");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.platform_user_id, "unknown");
    }

    #[test]
    fn test_map_platform_name_long_custom_truncated() {
        let long_name = "x".repeat(300);
        let ft = map_platform_name(&long_name);
        if let FrontendType::Custom(s) = ft {
            assert!(
                s.len() <= MAX_PLATFORM_NAME_BYTES,
                "custom platform name should be truncated to {MAX_PLATFORM_NAME_BYTES}, got {}",
                s.len()
            );
        } else {
            panic!("expected Custom variant");
        }
    }

    #[test]
    fn test_inbound_message_string_content_size_limit() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let huge_string = "x".repeat(MAX_NOTIFICATION_PAYLOAD_BYTES + 1);
        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "content": huge_string,
            "context": {}
        });
        handler.handle_inbound_message(Some(params));

        assert!(
            inbound_rx.try_recv().is_err(),
            "oversized string content should be rejected"
        );
    }

    #[test]
    fn test_inbound_message_missing_content() {
        let (handler, mut inbound_rx, _) = test_handler("test-plugin");

        let params = serde_json::json!({
            "pluginId": "test-plugin",
            "context": { "from": { "id": "user-1" } }
        });
        handler.handle_inbound_message(Some(params));

        assert!(
            inbound_rx.try_recv().is_err(),
            "missing content field should be rejected"
        );
    }

    #[test]
    fn test_register_channels_locally() {
        let handler =
            AstridClientHandler::new("plugin:test-plugin", Arc::new(CapabilitiesHandler::new()));

        let channels = vec![
            BridgeChannelInfo {
                name: "telegram".to_string(),
                definition: Some(BridgeChannelDefinition {
                    capabilities: Some(BridgeChannelCapabilities {
                        can_receive: true,
                        can_send: true,
                        ..Default::default()
                    }),
                }),
            },
            BridgeChannelInfo {
                name: "discord".to_string(),
                definition: None,
            },
        ];

        handler.register_channels_locally("test-plugin", &channels);

        let shared = handler
            .registered_connectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(shared.len(), 2);
        assert_eq!(shared[0].name, "telegram");
        assert_eq!(shared[1].name, "discord");
    }

    #[test]
    fn test_register_channels_locally_deduplicates() {
        let handler =
            AstridClientHandler::new("plugin:test-plugin", Arc::new(CapabilitiesHandler::new()));

        let channels = vec![BridgeChannelInfo {
            name: "telegram".to_string(),
            definition: None,
        }];

        handler.register_channels_locally("test-plugin", &channels);
        handler.register_channels_locally("test-plugin", &channels);

        let shared = handler
            .registered_connectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(shared.len(), 1, "duplicate channel should be deduplicated");
    }
}
