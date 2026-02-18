//! `impl rmcp::ClientHandler for AstridClientHandler`.
//!
//! Bridges the six rmcp client handler methods to the astrid capability types.

use std::collections::HashMap;

use astrid_core::{
    ElicitationAction as CoreElicitationAction, ElicitationRequest, UrlElicitationRequest,
};
use rmcp::model::{
    ClientCapabilities, ClientInfo, CreateElicitationRequestParams, CreateElicitationResult,
    CreateMessageRequestParams, CreateMessageResult, ElicitationAction as RmcpElicitationAction,
    ElicitationCapability, FormElicitationCapability, Implementation, ListRootsResult, Role,
    RootsCapabilities, SamplingCapability, SamplingMessageContent, UrlElicitationCapability,
};
use rmcp::service::{NotificationContext, RequestContext, RoleClient};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::types::ToolDefinition;

use super::super::convert::{convert_rmcp_schema, wrap_response_value};
use super::super::roots::RootsRequest;
use super::super::sampling::{SamplingContent, SamplingMessage, SamplingRequest};
use super::bridge::{
    ConnectorRegisteredParams, MAX_CHANNEL_NAME_LEN, MAX_CHANNELS_PER_PLUGIN, is_valid_channel_name,
};
use super::handler::AstridClientHandler;
use super::notice::ServerNotice;

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
