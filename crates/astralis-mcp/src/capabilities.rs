//! MCP client capabilities handlers.
//!
//! These handlers implement client-side capabilities from the MCP Nov 2025 spec:
//! - Sampling: Server-initiated LLM calls
//! - Roots: Server inquiries about operational boundaries
//! - Elicitation: Server requests for user input (canonical types from `astralis-core`)
//! - URL Elicitation: OAuth flows, payments, credentials (canonical types from `astralis-core`)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// Canonical elicitation types from astralis-core (single source of truth).
use astralis_core::{
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
// are canonical types defined in `astralis-core::frontend`. The handler
// traits below use those types directly — no MCP-local duplicates.

/// Handler for server requests for user input.
///
/// Implementations receive canonical [`ElicitationRequest`] from `astralis-core`
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
/// Implementations receive canonical [`UrlElicitationRequest`] from `astralis-core`
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

/// Convert an rmcp elicitation request's schema to a core elicitation schema.
///
/// The rmcp schema is a JSON Schema object with typed properties, while the core
/// schema is a simple enum (`Text`/`Secret`/`Select`/`Confirm`). This does a
/// best-effort conversion based on the first property's type.
///
/// Returns `(core_schema, first_property_name)` where the property name is used
/// to wrap single-value responses back into the object format rmcp expects.
fn convert_rmcp_schema(
    params: &CreateElicitationRequestParams,
) -> (ElicitationSchema, Option<String>) {
    let schema = &params.requested_schema;
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
    AnnotateAble, ClientCapabilities, ClientInfo, CreateMessageRequestParams, CreateMessageResult,
    ElicitationCapability, Implementation, ProtocolVersion, RawContent, RootsCapabilities,
};
use rmcp::model::{CreateElicitationResult, ElicitationAction as RmcpElicitationAction};
use rmcp::model::{ListRootsResult, Role};
use rmcp::service::{NotificationContext, RequestContext, RoleClient};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::types::ToolDefinition;

/// Notification from a running MCP server about a state change.
///
/// Sent over an internal channel from `AstralisClientHandler` to `McpClient`
/// so that tools caches and other state can be updated without polling.
pub(crate) enum ServerNotice {
    /// The server pushed `notifications/tools/list_changed`; the handler has
    /// already re-fetched the tool list and attached it here.
    ToolsRefreshed {
        /// Name of the server whose tools changed.
        server_name: String,
        /// Updated tool list (already converted to `ToolDefinition`).
        tools: Vec<ToolDefinition>,
    },
}

/// Bridge between astralis capability handlers and the rmcp `ClientHandler` trait.
///
/// This is the handler passed to `rmcp::ServiceExt::serve()` when connecting
/// to an MCP server. It delegates server-initiated requests (sampling, roots,
/// elicitation) to the configured `CapabilitiesHandler`.
pub struct AstralisClientHandler {
    server_name: String,
    inner: Arc<CapabilitiesHandler>,
    /// Channel for pushing notifications (tools changed, etc.) back to the
    /// `McpClient`. `None` if the caller does not care about notifications.
    notice_tx: Option<mpsc::UnboundedSender<ServerNotice>>,
}

impl AstralisClientHandler {
    /// Create a new handler for a specific server connection.
    pub fn new(server_name: impl Into<String>, inner: Arc<CapabilitiesHandler>) -> Self {
        Self {
            server_name: server_name.into(),
            inner,
            notice_tx: None,
        }
    }

    /// Attach a notice sender so that `on_tool_list_changed` can push
    /// refreshed tools back to the `McpClient`.
    pub(crate) fn with_notice_tx(mut self, tx: mpsc::UnboundedSender<ServerNotice>) -> Self {
        self.notice_tx = Some(tx);
        self
    }
}

impl rmcp::ClientHandler for AstralisClientHandler {
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
                Some(serde_json::Map::new())
            } else {
                None
            },
            elicitation: if self.inner.has_elicitation() {
                Some(ElicitationCapability::default())
            } else {
                None
            },
            ..Default::default()
        };

        ClientInfo {
            meta: None,
            protocol_version: ProtocolVersion::default(),
            capabilities,
            client_info: Implementation {
                name: "astralis".to_string(),
                title: Some("Astralis Secure Agent Runtime".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
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

        // Convert rmcp SamplingMessages to astralis SamplingMessages
        let messages = params
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                let content = match &*m.content {
                    RawContent::Text(t) => SamplingContent::Text {
                        text: t.text.clone(),
                    },
                    RawContent::Image(i) => SamplingContent::Image {
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
            message: rmcp::model::SamplingMessage {
                role: Role::Assistant,
                content: RawContent::text(text).no_annotation(),
            },
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
        let Some(ref handler) = self.inner.elicitation else {
            return Err(rmcp::ErrorData::internal_error(
                "Elicitation not supported",
                None,
            ));
        };

        // Convert rmcp schema to core schema
        let (core_schema, prop_name) = convert_rmcp_schema(&request);

        // Determine if required from schema's required field list
        let required = request
            .requested_schema
            .required
            .as_ref()
            .is_some_and(|r| !r.is_empty());

        let core_request =
            ElicitationRequest::new(&self.server_name, &request.message).with_schema(core_schema);
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

        let params = CreateElicitationRequestParams {
            meta: None,
            message: "Confirm?".to_string(),
            requested_schema: rmcp_schema,
        };

        let (schema, prop_name) = convert_rmcp_schema(&params);
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

        let params = CreateElicitationRequestParams {
            meta: None,
            message: "API key needed".to_string(),
            requested_schema: rmcp_schema,
        };

        let (schema, prop_name) = convert_rmcp_schema(&params);
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
}
