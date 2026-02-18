//! `ServerNotice` — in-process notifications from an MCP server connection.

use crate::types::ToolDefinition;

use super::bridge::BridgeChannelInfo;

/// Maximum payload size for custom notifications (1 MB).
///
/// Checked against re-serialized JSON, which may differ from wire size
/// (e.g. due to Unicode escape compression). This is a best-effort
/// post-parse heuristic; the true wire-level bound would require
/// transport-layer enforcement.
pub(super) const MAX_NOTIFICATION_PAYLOAD_BYTES: usize = 1_024 * 1_024;

/// Maximum length for a platform user ID (512 bytes, truncated at
/// a valid UTF-8 character boundary).
pub(super) const MAX_PLATFORM_USER_ID_BYTES: usize = 512;

/// Maximum size for the opaque context JSON payload in inbound messages (64 KB).
pub(super) const MAX_CONTEXT_BYTES: usize = 64 * 1024;

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
