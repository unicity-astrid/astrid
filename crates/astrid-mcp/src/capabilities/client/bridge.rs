//! Bridge channel types.
//!
//! These types are deserialized from untrusted plugin subprocess output.
//! All fields must be validated before use.

use astrid_core::MAX_CONNECTORS_PER_PLUGIN;
use serde::Deserialize;

/// Maximum channels a single plugin can register (bounds memory from untrusted plugins).
pub(super) const MAX_CHANNELS_PER_PLUGIN: usize = MAX_CONNECTORS_PER_PLUGIN;
/// Maximum length of a channel name in bytes.
pub(super) const MAX_CHANNEL_NAME_LEN: usize = 128;

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
pub(super) struct ConnectorRegisteredParams {
    #[serde(rename = "pluginId")]
    pub(super) plugin_id: String,
    pub(super) channels: Vec<BridgeChannelInfo>,
}

/// Returns `true` if the channel name contains only safe characters.
///
/// Allowed: ASCII alphanumeric, hyphens, underscores. Must be non-empty.
/// This prevents path traversal, null bytes, shell metacharacters, and
/// Unicode lookalikes from reaching downstream code.
pub(super) fn is_valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
