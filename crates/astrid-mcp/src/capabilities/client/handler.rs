//! `AstridClientHandler` — bridges astrid capability handlers with rmcp.

use std::sync::{Arc, Mutex, PoisonError};

use astrid_core::{
    ConnectorCapabilities, ConnectorDescriptor, ConnectorId, ConnectorProfile, ConnectorSource,
    FrontendType, InboundMessage,
};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::super::handler::CapabilitiesHandler;
use super::bridge::BridgeChannelInfo;
use super::helpers::{
    estimate_json_size, extract_inbound_content, extract_platform_user_id, map_platform_name,
};
use super::notice::{
    MAX_CONTEXT_BYTES, MAX_NOTIFICATION_PAYLOAD_BYTES, MAX_PLATFORM_USER_ID_BYTES, ServerNotice,
};

/// Bridge between astrid capability handlers and the rmcp `ClientHandler` trait.
///
/// This is the handler passed to `rmcp::ServiceExt::serve()` when connecting
/// to an MCP server. It delegates server-initiated requests (sampling, roots,
/// elicitation) to the configured `CapabilitiesHandler`.
pub struct AstridClientHandler {
    pub(super) server_name: String,
    pub(super) inner: Arc<CapabilitiesHandler>,
    /// Channel for pushing notifications (tools changed, etc.) back to the
    /// `McpClient`. `None` if the caller does not care about notifications.
    pub(super) notice_tx: Option<mpsc::UnboundedSender<ServerNotice>>,
    /// Plugin ID for anti-spoofing validation on inbound notifications.
    pub(super) plugin_id: String,
    /// Channel for inbound messages from the bridge.
    /// Bounded to 256 (set by caller in `McpPlugin::load()`).
    pub(super) inbound_tx: Option<mpsc::Sender<InboundMessage>>,
    /// Shared registered connectors for connector ID lookups on inbound messages.
    ///
    /// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because this is only
    /// accessed in non-async notification handlers and updated during connector
    /// registration. The lock is never held across an `.await` point, so a
    /// blocking mutex is correct and avoids the overhead of an async-aware mutex.
    pub(super) registered_connectors: Arc<Mutex<Vec<ConnectorDescriptor>>>,
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
    pub(super) fn handle_inbound_message(&self, params: Option<Value>) {
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

        let Some(content) = extract_inbound_content(&params, MAX_NOTIFICATION_PAYLOAD_BYTES) else {
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
        let platform_user_id = extract_platform_user_id(&msg_context, MAX_PLATFORM_USER_ID_BYTES);

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
    pub(super) fn register_channels_locally(
        &self,
        plugin_id: &str,
        channels: &[BridgeChannelInfo],
    ) {
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, PoisonError};

    use super::super::super::handler::CapabilitiesHandler;
    use super::super::bridge::{
        BridgeChannelCapabilities, BridgeChannelDefinition, BridgeChannelInfo,
    };
    use super::AstridClientHandler;

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
