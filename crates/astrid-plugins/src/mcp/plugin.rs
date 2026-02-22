//! MCP-backed plugin implementation.
//!
//! [`McpPlugin`] wraps an MCP server child process as a [`Plugin`], exposing
//! the server's tools as [`PluginTool`] instances.  The child process is
//! spawned and connected during [`Plugin::load()`], and gracefully shut down
//! during [`Plugin::unload()`].
//!
//! Security is enforced at the runtime layer: the
//! [`SecurityInterceptor`](astrid_approval::SecurityInterceptor) checks
//! capability tokens for `SensitiveAction::McpToolCall` before the tool's
//! `execute()` is ever called. OS-level sandboxing is handled by
//! [`SandboxProfile`](crate::sandbox::SandboxProfile).
//!
//! # Hook Event Forwarding
//!
//! The runtime can push hook events to the plugin's MCP server via
//! [`McpPlugin::send_hook_event()`], which sends a custom notification
//! (`notifications/astrid.hookEvent`) over the MCP connection.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use rmcp::ServiceExt;
use rmcp::model::{ClientNotification, CustomNotification};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use astrid_core::connector::{ConnectorCapabilities, ConnectorDescriptor, ConnectorSource};
use astrid_core::identity::FrontendType;
use astrid_core::{HookEvent, InboundMessage};
use astrid_mcp::{
    AstridClientHandler, BridgeChannelInfo, CapabilitiesHandler, McpClient, ServerNotice,
};

use crate::context::PluginContext;
use crate::error::{PluginError, PluginResult};
use crate::manifest::{PluginCapability, PluginEntryPoint, PluginManifest};
use crate::mcp::protocol::McpProtocolConnection;
use crate::mcp::state::McpConnection;
use crate::mcp::tool::McpPluginTool;
use crate::plugin::{Plugin, PluginId, PluginState};
use crate::sandbox::SandboxProfile;
use crate::tool::PluginTool;

/// Timeout for graceful shutdown of plugin MCP servers.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Type alias for the running MCP service backing a plugin.
type PluginMcpService = RunningService<RoleClient, AstridClientHandler>;

/// A plugin backed by an MCP server child process.
///
/// The MCP server is spawned during `load()` and shut down during
/// `unload()`.  Tool calls are forwarded over the MCP connection via
/// the stored [`Peer`] handle.
///
/// # Dependency Injection
///
/// `McpPlugin::new()` receives an [`McpClient`] at construction time.
/// The caller (plugin manager / runtime) decides which `Plugin` impl to
/// create based on the manifest's [`PluginEntryPoint`].
pub struct McpPlugin {
    id: PluginId,
    manifest: PluginManifest,
    state: PluginState,
    tools: Vec<Arc<dyn PluginTool>>,
    /// MCP server name (format: `"plugin:{plugin_id}"`).
    server_name: String,
    /// Injected at construction — used for hook forwarding and lifecycle.
    mcp_client: McpClient,
    /// The active connection state for the MCP server.
    connection: Option<McpConnection>,
    /// Optional sandbox profile applied to the child process.
    sandbox: Option<SandboxProfile>,
    /// Plugin install directory — used as `current_dir` when spawning the
    /// subprocess so that relative paths in `args` resolve correctly.
    plugin_dir: Option<PathBuf>,
    /// Connectors registered by the bridge via `connectorRegistered` notification.
    registered_connectors: Vec<ConnectorDescriptor>,
    /// Receiver for server notices (connector registrations, tool refreshes).
    notice_rx: Option<tokio::sync::mpsc::UnboundedReceiver<ServerNotice>>,
    /// Keepalive clone of the inbound message sender — never used for
    /// sending. Its sole purpose is to prevent the receiver from seeing EOF
    /// until [`unload`] drops this clone. The active sender (the one that
    /// calls `try_send`) lives in [`AstridClientHandler`]; when the MCP
    /// session is closed, that sender is dropped first, and then this
    /// keepalive is set to `None` in `unload()`.
    inbound_tx: Option<mpsc::Sender<InboundMessage>>,
    /// Receiver for inbound messages from the bridge.
    inbound_rx: Option<mpsc::Receiver<InboundMessage>>,
    /// Shared with `AstridClientHandler` for connector ID lookups on inbound messages.
    ///
    /// Uses `std::sync::Mutex` — see the matching doc on `AstridClientHandler`'s
    /// field for rationale. Must not be held across `.await` boundaries.
    shared_connectors: Arc<Mutex<Vec<ConnectorDescriptor>>>,
}

impl McpPlugin {
    /// Create a new MCP plugin.
    ///
    /// The plugin starts in `Unloaded` state. Call [`Plugin::load()`] to
    /// spawn the MCP server and discover tools.
    #[must_use]
    pub fn new(manifest: PluginManifest, mcp_client: McpClient) -> Self {
        let id = manifest.id.clone();
        let server_name = format!("plugin:{id}");
        Self {
            id,
            manifest,
            state: PluginState::Unloaded,
            tools: Vec::new(),
            server_name,
            mcp_client,
            connection: None,
            sandbox: None,
            plugin_dir: None,
            registered_connectors: Vec::new(),
            notice_rx: None,
            inbound_tx: None,
            inbound_rx: None,
            shared_connectors: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Set the plugin install directory. Used as `current_dir` when spawning
    /// the subprocess so relative paths in args resolve correctly.
    #[must_use]
    pub fn with_plugin_dir(mut self, dir: PathBuf) -> Self {
        self.plugin_dir = Some(dir);
        self
    }

    /// Set an OS sandbox profile for the child process.
    #[must_use]
    pub fn with_sandbox(mut self, profile: SandboxProfile) -> Self {
        self.sandbox = Some(profile);
        self
    }

    /// Get the MCP server name for this plugin.
    #[must_use]
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Get a reference to the injected [`McpClient`].
    #[must_use]
    pub fn mcp_client(&self) -> &McpClient {
        &self.mcp_client
    }

    /// Check if the MCP server child process is still running.
    ///
    /// Returns `true` if the plugin is loaded and the underlying MCP service
    /// reports it is still alive. If the process has crashed, transitions the
    /// plugin state to `Failed` and returns `false`.
    pub fn check_health(&mut self) -> bool {
        if !matches!(self.state, PluginState::Ready) {
            return false;
        }

        let alive = self
            .connection
            .as_ref()
            .is_some_and(super::state::McpConnection::is_alive);

        if !alive {
            let msg = "MCP server process exited unexpectedly".to_string();
            warn!(plugin_id = %self.id, "{msg}");
            self.state = PluginState::Failed(msg);
            self.tools.clear();
            // Drop the connection to release the AstridClientHandler's Arc
            // clone of shared_connectors, minimizing phantom entries from
            // buffered notifications arriving after we clear state.
            self.connection = None;
            // Clear connector state so stale connectors are not visible
            self.registered_connectors.clear();
            self.shared_connectors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            self.notice_rx = None;
            self.inbound_tx = None;
            self.inbound_rx = None;
        }

        alive
    }

    /// Take ownership of the inbound message receiver.
    ///
    /// This is a one-time transfer — subsequent calls return `None`.
    /// Matches the WASM plugin pattern.
    ///
    /// Must be called **after** [`load()`] — the receiver is only created
    /// during load when the manifest declares `PluginCapability::Connector`.
    pub fn take_inbound_rx(&mut self) -> Option<mpsc::Receiver<InboundMessage>> {
        self.inbound_rx.take()
    }

    /// Drain pending connector notices and return the updated connector list.
    ///
    /// Lazily drains any pending connector registration notices that may have
    /// arrived since the last call, ensuring callers always see the latest
    /// state regardless of notification delivery timing.
    ///
    /// Use this over [`Plugin::connectors()`] when you have a `&mut` reference
    /// and want to pick up late-arriving notifications.
    pub fn refresh_connectors(&mut self) -> &[ConnectorDescriptor] {
        self.drain_connector_notices();
        &self.registered_connectors
    }

    /// Non-blockingly drain all pending `ConnectorsRegistered` notices from
    /// the notice channel and convert them to `ConnectorDescriptor`s.
    fn drain_connector_notices(&mut self) {
        // Collect all channel infos first to avoid borrow conflict.
        let mut all_channels = Vec::new();
        if let Some(rx) = &mut self.notice_rx {
            while let Ok(notice) = rx.try_recv() {
                match notice {
                    ServerNotice::ConnectorsRegistered { channels, .. } => {
                        all_channels.extend(channels);
                    },
                    ServerNotice::ToolsRefreshed { server_name, .. } => {
                        debug!(
                            server = %server_name,
                            "Discarded ToolsRefreshed notice during connector drain"
                        );
                    },
                }
            }
        }
        for ch in &all_channels {
            if let Some(desc) = self.channel_to_descriptor(ch) {
                // Deduplicate: skip if a connector with the same name is already registered.
                if !self
                    .registered_connectors
                    .iter()
                    .any(|d| d.name == desc.name)
                {
                    self.registered_connectors.push(desc.clone());
                    // Also push to shared state for inbound message routing.
                    let mut shared = self
                        .shared_connectors
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    if !shared.iter().any(|d| d.name == desc.name) {
                        shared.push(desc);
                    }
                }
            }
        }
    }

    /// Convert a `BridgeChannelInfo` to a `ConnectorDescriptor`.
    ///
    /// Capabilities are parsed from the plugin's definition. If no capabilities
    /// are declared, defaults to `receive_only` (least privilege).
    fn channel_to_descriptor(&self, ch: &BridgeChannelInfo) -> Option<ConnectorDescriptor> {
        let frontend_type = match ch.name.to_lowercase().as_str() {
            "telegram" => FrontendType::Telegram,
            "discord" => FrontendType::Discord,
            "slack" => FrontendType::Slack,
            "whatsapp" => FrontendType::WhatsApp,
            "web" => FrontendType::Web,
            "cli" => FrontendType::Cli,
            _ => FrontendType::Custom(ch.name.clone()),
        };

        let source = ConnectorSource::new_openclaw(self.id.as_str())
            .map_err(|e| {
                warn!(
                    plugin_id = %self.id,
                    channel = %ch.name,
                    error = %e,
                    "Failed to create ConnectorSource for channel"
                );
                e
            })
            .ok()?;

        // Parse capabilities from the plugin's definition, defaulting to
        // receive_only (least privilege) if none are declared.
        let capabilities = ch
            .definition
            .as_ref()
            .and_then(|d| d.capabilities.as_ref())
            .map_or_else(ConnectorCapabilities::receive_only, |c| {
                ConnectorCapabilities {
                    can_receive: c.can_receive,
                    can_send: c.can_send,
                    can_approve: c.can_approve,
                    can_elicit: c.can_elicit,
                    supports_rich_media: c.supports_rich_media,
                    supports_threads: c.supports_threads,
                    supports_buttons: c.supports_buttons,
                }
            });

        Some(
            ConnectorDescriptor::builder(&ch.name, frontend_type)
                .source(source)
                .capabilities(capabilities)
                .build(),
        )
    }

    /// Send plugin config as a post-init notification. The MCP handshake
    /// doesn't support custom initialization options, so we deliver config
    /// via a custom notification after the bridge's dispatch loop is running.
    async fn send_plugin_config(&self, ctx: &PluginContext, peer: &Peer<RoleClient>) {
        if ctx.config.is_empty() {
            return;
        }

        let notification = CustomNotification::new(
            "notifications/astrid.setPluginConfig",
            Some(serde_json::json!({ "config": ctx.config })),
        );
        if let Err(e) = peer
            .send_notification(ClientNotification::CustomNotification(notification))
            .await
        {
            warn!(
                plugin_id = %self.id,
                error = %e,
                "Failed to send plugin config notification"
            );
        } else {
            debug!(
                plugin_id = %self.id,
                config_keys = ?ctx.config.keys().collect::<Vec<_>>(),
                "Sent plugin config to bridge"
            );
        }
    }

    /// Build the `tokio::process::Command` from the manifest entry point,
    /// optionally applying sandbox wrapping.
    ///
    /// On Linux with a sandbox profile, this applies Landlock rules via a
    /// `pre_exec` hook. The `unsafe` is required by POSIX: `pre_exec` runs
    /// between `fork()` and `exec()` where only async-signal-safe operations
    /// are permitted. The Landlock syscalls used here are safe in practice
    /// (they are simple kernel calls that don't allocate or lock).
    #[allow(unsafe_code)]
    fn build_command(&self) -> PluginResult<tokio::process::Command> {
        let PluginEntryPoint::Mcp {
            command,
            args,
            env,
            binary_hash: _,
        } = &self.manifest.entry_point
        else {
            return Err(PluginError::UnsupportedEntryPoint(
                "expected Mcp entry point".into(),
            ));
        };

        // Optionally wrap with sandbox
        let (final_cmd, final_args) = if let Some(sandbox) = &self.sandbox {
            sandbox.wrap_command(command, args)?
        } else {
            (command.clone(), args.clone())
        };

        let mut cmd = tokio::process::Command::new(&final_cmd);
        cmd.args(&final_args);

        // Set working directory so relative paths in args resolve correctly.
        if let Some(dir) = &self.plugin_dir {
            cmd.current_dir(dir);
        }

        for (key, value) in env {
            if astrid_core::env_policy::is_blocked_spawn_env(key) {
                warn!(
                    plugin_id = %self.manifest.id,
                    key = %key,
                    "Ignoring blocked env var from plugin manifest \
                     (may compromise sandbox isolation)"
                );
                continue;
            }
            cmd.env(key, value);
        }

        // Inject HOME override from sandbox profile (Tier 2 plugins)
        if let Some(sandbox) = &self.sandbox
            && let Some(home) = sandbox.home_override()
        {
            cmd.env("HOME", home);
        }

        // On Linux, apply Landlock rules via pre_exec hook.
        // PathFds are opened HERE (before fork) where heap allocation is safe.
        // Only raw Landlock syscalls run inside the pre_exec closure.
        #[cfg(target_os = "linux")]
        if let Some(sandbox) = &self.sandbox {
            let prepared = crate::mcp::platform::prepare_landlock_rules(&sandbox.landlock_rules());
            let mut prepared = Some(prepared);
            let rlimits = sandbox.resource_limits.clone();
            // SAFETY: pre_exec runs between fork() and exec(). The closure
            // only invokes Landlock syscalls (landlock_create_ruleset,
            // landlock_add_rule, landlock_restrict_self) and setrlimit using
            // pre-opened file descriptors. Error paths use last_os_error()
            // (reads errno, no heap allocation). The ok_or_else and map_err
            // closures in the Landlock path may allocate on error — this is
            // technically not async-signal-safe but acceptable since errors
            // here are fatal (process will not exec).
            unsafe {
                cmd.pre_exec(move || {
                    // Apply Landlock filesystem restrictions
                    let rules = prepared.take().ok_or_else(|| {
                        std::io::Error::other("Landlock pre_exec called more than once")
                    })?;
                    crate::mcp::platform::enforce_landlock_rules(rules).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.clone())
                    })?;

                    // Apply resource limits if configured
                    if let Some(ref limits) = rlimits {
                        crate::mcp::platform::apply_resource_limits(limits)?;
                    }

                    Ok(())
                });
            }
        }

        Ok(cmd)
    }

    /// Verify the binary hash if configured in the manifest.
    fn verify_binary_hash(&self) -> PluginResult<()> {
        let PluginEntryPoint::Mcp {
            command,
            binary_hash: Some(expected),
            ..
        } = &self.manifest.entry_point
        else {
            return Ok(());
        };

        // Use the same verification logic as ServerConfig
        let binary_path = which::which(command).map_err(|e| PluginError::McpServerFailed {
            plugin_id: self.id.clone(),
            message: format!("Cannot find binary {command}: {e}"),
        })?;

        let binary_data = std::fs::read(&binary_path)?;
        let actual_hash = astrid_crypto::ContentHash::hash(&binary_data);
        let actual_str = format!("sha256:{}", actual_hash.to_hex());

        if expected != &actual_str {
            return Err(PluginError::McpServerFailed {
                plugin_id: self.id.clone(),
                message: format!("Binary hash mismatch: expected {expected}, got {actual_str}"),
            });
        }

        Ok(())
    }
}

#[async_trait]
impl Plugin for McpPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn state(&self) -> PluginState {
        self.state.clone()
    }

    async fn load(&mut self, ctx: &PluginContext) -> PluginResult<()> {
        self.state = PluginState::Loading;

        // Defensive clear: ensure no stale state from a previous load attempt
        // (e.g., handshake succeeded but tool discovery failed, leaving stale
        // entries in shared_connectors from the previous handler).
        self.registered_connectors.clear();
        self.shared_connectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();

        // 1. Verify binary hash if configured
        if let Err(e) = self.verify_binary_hash() {
            self.state = PluginState::Failed(e.to_string());
            return Err(e);
        }

        // 2. Build the command
        let cmd = match self.build_command() {
            Ok(cmd) => cmd,
            Err(e) => {
                self.state = PluginState::Failed(e.to_string());
                return Err(e);
            },
        };

        // 3. Create transport (spawns the child process)
        let transport = TokioChildProcess::new(cmd).map_err(|e| {
            let err = PluginError::McpServerFailed {
                plugin_id: self.id.clone(),
                message: format!("Failed to spawn MCP server process: {e}"),
            };
            self.state = PluginState::Failed(err.to_string());
            err
        })?;

        // 4. MCP handshake
        let has_connector_cap = self
            .manifest
            .capabilities
            .iter()
            .any(|c| matches!(c, PluginCapability::Connector { .. }));

        let handler = Arc::new(CapabilitiesHandler::new());

        // Notice channel: used for connector registrations (notice-based flow).
        let (notice_tx, notice_rx) =
            tokio::sync::mpsc::unbounded_channel::<astrid_mcp::ServerNotice>();

        // Create inbound channel if this plugin declares the Connector capability.
        // The tx side goes into AstridClientHandler; rx and keepalive tx are stored on
        // `self` only AFTER the handshake succeeds (see below) so that failed load
        // attempts do not leave stale channels on the plugin.
        let inbound_channels = if has_connector_cap {
            let (inbound_tx, inbound_rx) = mpsc::channel(256);
            Some((inbound_tx, inbound_rx))
        } else {
            None
        };

        let mut client_handler = AstridClientHandler::new(&self.server_name, handler)
            .with_notice_tx(notice_tx)
            .with_plugin_id(self.id.as_str());

        if let Some((ref inbound_tx, _)) = inbound_channels {
            client_handler = client_handler
                .with_inbound_tx(inbound_tx.clone())
                .with_shared_connectors(Arc::clone(&self.shared_connectors));
        }

        let service: PluginMcpService = client_handler.serve(transport).await.map_err(|e| {
            let err = PluginError::McpServerFailed {
                plugin_id: self.id.clone(),
                message: format!("MCP handshake failed: {e}"),
            };
            self.state = PluginState::Failed(err.to_string());
            err
        })?;

        // Store notice_rx now that the handshake succeeded.
        self.notice_rx = Some(notice_rx);

        // 5. Discover tools
        let rmcp_tools = service.list_all_tools().await.map_err(|e| {
            let err = PluginError::McpServerFailed {
                plugin_id: self.id.clone(),
                message: format!("Failed to list tools: {e}"),
            };
            self.state = PluginState::Failed(err.to_string());
            self.notice_rx = None;
            self.registered_connectors.clear();
            err
        })?;

        // 6. Extract peer handle
        let peer = service.peer().clone();

        // 7. Create McpPluginTool wrappers
        let tools: Vec<Arc<dyn PluginTool>> = rmcp_tools
            .iter()
            .map(|t| {
                Arc::new(McpPluginTool {
                    name: t.name.to_string(),
                    description: t.description.as_deref().unwrap_or("").to_string(),
                    input_schema: serde_json::to_value(&*t.input_schema)
                        .unwrap_or_else(|_| serde_json::json!({"type": "object"})),
                    server_name: self.server_name.clone(),
                    peer: peer.clone(),
                }) as Arc<dyn PluginTool>
            })
            .collect();

        self.connection = Some(McpConnection::new(self.id.clone(), service, peer.clone()));
        self.tools = tools;

        // Store inbound channels now that the connection is established.
        if let Some((inbound_tx, inbound_rx)) = inbound_channels {
            self.inbound_tx = Some(inbound_tx);
            self.inbound_rx = Some(inbound_rx);
        }

        // 8. Send plugin config + drain connector registration notices.
        // Config and drain happen BEFORE setting Ready so that connectors()
        // is populated by the time the runtime queries it.
        self.send_plugin_config(ctx, &peer).await;

        // Yield to let the bridge process the config notification and
        // fire any connectorRegistered notifications back to us.
        tokio::task::yield_now().await;

        // Drain any connector registration notices that arrived during
        // the handshake (bridge sends them right after `initialized`).
        self.drain_connector_notices();

        info!(
            plugin_id = %self.id,
            server_name = %self.server_name,
            tool_count = self.tools.len(),
            connector_count = self.registered_connectors.len(),
            "MCP plugin loaded successfully"
        );

        self.state = PluginState::Ready;

        Ok(())
    }

    async fn unload(&mut self) -> PluginResult<()> {
        self.state = PluginState::Unloading;

        self.tools.clear();

        // Gracefully close the MCP session BEFORE clearing connector state
        // so that in-flight notifications from the bridge can still be
        // received and processed by the handler.
        if let Some(mut connection) = self.connection.take() {
            let _ = connection.close(SHUTDOWN_TIMEOUT).await;
        }

        // Now clear connector state after the MCP session is closed
        self.registered_connectors.clear();
        {
            let mut shared = self
                .shared_connectors
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            shared.clear();
        }
        self.notice_rx = None;
        self.inbound_tx = None;
        self.inbound_rx = None;

        self.state = PluginState::Unloaded;

        info!(plugin_id = %self.id, "MCP plugin unloaded");

        Ok(())
    }

    fn tools(&self) -> &[Arc<dyn PluginTool>] {
        &self.tools
    }

    fn connectors(&self) -> &[ConnectorDescriptor] {
        &self.registered_connectors
    }

    /// Send a hook event notification to the MCP server.
    ///
    /// Sends a custom MCP notification with method
    /// `notifications/astrid.hookEvent`. This is fire-and-forget;
    /// errors are logged but do not propagate.
    async fn send_hook_event(&self, event: HookEvent, data: Value) {
        if let Some(connection) = &self.connection {
            connection.send_hook_event(event, data).await;
        } else {
            debug!(
                plugin_id = %self.id,
                "Cannot send hook event: no peer connection"
            );
        }
    }
}

/// Create a plugin from a manifest, choosing the appropriate implementation
/// based on the entry point type.
///
/// # Errors
///
/// - [`PluginError::McpClientRequired`] if the entry point is `Mcp` but
///   no `McpClient` was provided.
/// - [`PluginError::UnsupportedEntryPoint`] if the entry point type is
///   not supported (e.g. `Wasm` — handled by a different subsystem).
pub fn create_plugin(
    manifest: PluginManifest,
    mcp_client: Option<McpClient>,
    plugin_dir: Option<PathBuf>,
) -> PluginResult<Box<dyn Plugin>> {
    match &manifest.entry_point {
        PluginEntryPoint::Wasm { .. } => Err(PluginError::UnsupportedEntryPoint("wasm".into())),
        PluginEntryPoint::Mcp { .. } => {
            let client = mcp_client.ok_or(PluginError::McpClientRequired)?;
            let mut plugin = McpPlugin::new(manifest, client);
            if let Some(dir) = plugin_dir {
                plugin = plugin.with_plugin_dir(dir);
            }
            Ok(Box::new(plugin))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn mcp_manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: PluginId::from_static(id),
            name: format!("Test MCP Plugin {id}"),
            version: "0.1.0".into(),
            description: Some("Test MCP plugin".into()),
            author: None,
            entry_point: PluginEntryPoint::Mcp {
                command: "node".into(),
                args: vec!["dist/index.js".into()],
                env: HashMap::new(),
                binary_hash: None,
            },
            capabilities: vec![],
            connectors: vec![],
            config: HashMap::new(),
        }
    }

    fn wasm_manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: PluginId::from_static(id),
            name: format!("Test WASM Plugin {id}"),
            version: "0.1.0".into(),
            description: None,
            author: None,
            entry_point: PluginEntryPoint::Wasm {
                path: "plugin.wasm".into(),
                hash: None,
            },
            capabilities: vec![],
            connectors: vec![],
            config: HashMap::new(),
        }
    }

    fn connector_manifest(id: &str) -> PluginManifest {
        let mut m = mcp_manifest(id);
        m.name = format!("Test Connector Plugin {id}");
        m.description = Some("Test connector plugin".into());
        m.capabilities = vec![PluginCapability::Connector {
            profile: astrid_core::ConnectorProfile::Bridge,
        }];
        m
    }

    fn test_mcp_client() -> McpClient {
        McpClient::with_config(astrid_mcp::ServersConfig::default())
    }

    #[tokio::test]
    async fn test_mcp_plugin_creation() {
        let manifest = mcp_manifest("test-mcp");
        let client = test_mcp_client();
        let plugin = McpPlugin::new(manifest, client);

        assert_eq!(plugin.id().as_str(), "test-mcp");
        assert_eq!(plugin.state(), PluginState::Unloaded);
        assert!(plugin.tools().is_empty());
        assert_eq!(plugin.server_name(), "plugin:test-mcp");
    }

    #[tokio::test]
    async fn test_mcp_plugin_with_sandbox() {
        let manifest = mcp_manifest("test-mcp");
        let client = test_mcp_client();
        let sandbox = SandboxProfile::new("/workspace".into(), "/plugins/test".into());
        let plugin = McpPlugin::new(manifest, client).with_sandbox(sandbox);

        assert!(plugin.sandbox.is_some());
    }

    #[tokio::test]
    async fn test_create_plugin_mcp() {
        let manifest = mcp_manifest("test-mcp");
        let client = test_mcp_client();
        let plugin = create_plugin(manifest, Some(client), None);
        assert!(plugin.is_ok());
    }

    #[test]
    fn test_create_plugin_mcp_requires_client() {
        let manifest = mcp_manifest("test-mcp");
        let result = create_plugin(manifest, None, None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::McpClientRequired
        ));
    }

    #[test]
    fn test_create_plugin_wasm_unsupported() {
        let manifest = wasm_manifest("test-wasm");
        let result = create_plugin(manifest, None, None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::UnsupportedEntryPoint(_)
        ));
    }

    #[tokio::test]
    async fn test_server_name_format() {
        let manifest = mcp_manifest("my-cool-plugin");
        let client = test_mcp_client();
        let plugin = McpPlugin::new(manifest, client);
        assert_eq!(plugin.server_name(), "plugin:my-cool-plugin");
    }

    #[tokio::test]
    async fn test_health_check_unloaded_returns_false() {
        let manifest = mcp_manifest("test-health");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);
        assert!(!plugin.check_health());
    }

    /// Verify that rmcp can deserialize the bridge's tools/list response format.
    #[test]
    fn test_bridge_tools_list_deserialization() {
        use rmcp::model::*;

        let json = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"__astrid_get_agent_context","description":"Returns plugin context","inputSchema":{"type":"object","properties":{}}}]}}"#;

        let msg: ServerJsonRpcMessage = match serde_json::from_str(json) {
            Ok(m) => m,
            Err(e) => panic!("Failed to deserialize bridge response: {e}"),
        };
        match msg {
            JsonRpcMessage::Response(resp) => match resp.result {
                ServerResult::ListToolsResult(r) => {
                    assert_eq!(r.tools.len(), 1);
                    assert_eq!(r.tools[0].name.as_ref(), "__astrid_get_agent_context");
                },
                _other => panic!("Expected ListToolsResult variant"),
            },
            _ => panic!("Expected Response variant"),
        }
    }

    /// Verify multi-tool response with complex schemas deserializes correctly.
    #[test]
    fn test_bridge_multi_tool_deserialization() {
        use rmcp::model::*;

        // Simulates a plugin that registers several tools with varied schemas.
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[
            {"name":"send_token","description":"Send a token","inputSchema":{
                "type":"object",
                "properties":{"to":{"type":"string"},"amount":{"type":"number"}},
                "required":["to","amount"]
            }},
            {"name":"get_balance","description":"","inputSchema":{"type":"object"}},
            {"name":"__astrid_get_agent_context","description":"Returns plugin context","inputSchema":{"type":"object","properties":{}}}
        ]}}"#;

        let msg: ServerJsonRpcMessage =
            serde_json::from_str(json).expect("Failed to deserialize multi-tool response");
        match msg {
            JsonRpcMessage::Response(resp) => match resp.result {
                ServerResult::ListToolsResult(r) => {
                    assert_eq!(r.tools.len(), 3);
                    assert_eq!(r.tools[0].name.as_ref(), "send_token");
                    assert_eq!(r.tools[1].name.as_ref(), "get_balance");
                },
                other => panic!("Expected ListToolsResult, got {other:?}"),
            },
            other => panic!("Expected Response, got {other:?}"),
        }
    }

    /// Verify that bad inputSchema types cause `CustomResult` fallthrough.
    /// This documents the failure mode we're guarding against in the bridge.
    #[test]
    fn test_array_input_schema_falls_to_custom_result() {
        use rmcp::model::*;

        // inputSchema as array — rmcp Tool requires JsonObject (Map<String, Value>)
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[
            {"name":"bad_tool","description":"","inputSchema":["string"]}
        ]}}"#;

        let msg: ServerJsonRpcMessage =
            serde_json::from_str(json).expect("Should still parse as JsonRpcMessage");
        match msg {
            JsonRpcMessage::Response(resp) => {
                // Should NOT be ListToolsResult because inputSchema is an array
                assert!(
                    !matches!(resp.result, ServerResult::ListToolsResult(_)),
                    "Array inputSchema should not parse as ListToolsResult"
                );
            },
            other => panic!("Expected Response, got {other:?}"),
        }
    }

    /// Verify that non-string description causes `CustomResult` fallthrough.
    #[test]
    fn test_numeric_description_falls_to_custom_result() {
        use rmcp::model::*;

        // description as number — rmcp Tool expects Option<Cow<str>>
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[
            {"name":"bad_tool","description":42,"inputSchema":{"type":"object"}}
        ]}}"#;

        let msg: ServerJsonRpcMessage =
            serde_json::from_str(json).expect("Should still parse as JsonRpcMessage");
        match msg {
            JsonRpcMessage::Response(resp) => {
                assert!(
                    !matches!(resp.result, ServerResult::ListToolsResult(_)),
                    "Numeric description should not parse as ListToolsResult"
                );
            },
            other => panic!("Expected Response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_connectors_returns_registered() {
        let manifest = connector_manifest("test-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        // Initially empty
        assert!(plugin.connectors().is_empty());

        // Manually push descriptors (simulating drain)
        let desc = ConnectorDescriptor::builder("telegram", FrontendType::Telegram)
            .source(ConnectorSource::new_openclaw("test-conn").unwrap())
            .profile(astrid_core::ConnectorProfile::Bridge)
            .build();
        plugin.registered_connectors.push(desc);

        assert_eq!(plugin.connectors().len(), 1);
        assert_eq!(plugin.connectors()[0].name, "telegram");
        assert!(matches!(
            plugin.connectors()[0].frontend_type,
            FrontendType::Telegram
        ));
        assert!(matches!(
            plugin.connectors()[0].profile,
            astrid_core::ConnectorProfile::Bridge
        ));
    }

    #[tokio::test]
    async fn test_take_inbound_rx() {
        let manifest = connector_manifest("test-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        // Before load, no rx
        assert!(plugin.take_inbound_rx().is_none());

        // Manually set one up
        let (tx, rx) = mpsc::channel(256);
        plugin.inbound_rx = Some(rx);
        plugin.inbound_tx = Some(tx);

        // First take returns Some
        assert!(plugin.take_inbound_rx().is_some());
        // Second take returns None
        assert!(plugin.take_inbound_rx().is_none());
    }

    #[tokio::test]
    async fn test_check_health_clears_connector_state() {
        let manifest = connector_manifest("test-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        // Simulate a loaded plugin with connector state but no service
        // (service = None simulates a dead child process)
        plugin.state = PluginState::Ready;
        let desc = ConnectorDescriptor::builder("telegram", FrontendType::Telegram)
            .source(ConnectorSource::new_openclaw("test-conn").unwrap())
            .build();
        plugin.registered_connectors.push(desc.clone());
        plugin.shared_connectors.lock().unwrap().push(desc);
        let (tx, rx) = mpsc::channel(256);
        plugin.inbound_tx = Some(tx);
        plugin.inbound_rx = Some(rx);
        let (_notice_tx, notice_rx) =
            tokio::sync::mpsc::unbounded_channel::<astrid_mcp::ServerNotice>();
        plugin.notice_rx = Some(notice_rx);

        // service is None -> not alive
        assert!(!plugin.check_health());
        assert!(matches!(plugin.state, PluginState::Failed(_)));
        assert!(plugin.registered_connectors.is_empty());
        assert!(plugin.shared_connectors.lock().unwrap().is_empty());
        assert!(plugin.inbound_tx.is_none());
        assert!(plugin.inbound_rx.is_none());
        assert!(plugin.notice_rx.is_none());
    }

    #[tokio::test]
    async fn test_unload_clears_connector_state() {
        let manifest = connector_manifest("test-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        // Populate connector state (no service to close)
        plugin.state = PluginState::Ready;
        let desc = ConnectorDescriptor::builder("discord", FrontendType::Discord)
            .source(ConnectorSource::new_openclaw("test-conn").unwrap())
            .build();
        plugin.registered_connectors.push(desc.clone());
        plugin.shared_connectors.lock().unwrap().push(desc);
        let (tx, rx) = mpsc::channel(256);
        plugin.inbound_tx = Some(tx);
        plugin.inbound_rx = Some(rx);
        let (_notice_tx, notice_rx) =
            tokio::sync::mpsc::unbounded_channel::<astrid_mcp::ServerNotice>();
        plugin.notice_rx = Some(notice_rx);

        plugin.unload().await.unwrap();

        assert_eq!(plugin.state(), PluginState::Unloaded);
        assert!(plugin.registered_connectors.is_empty());
        assert!(plugin.shared_connectors.lock().unwrap().is_empty());
        assert!(plugin.inbound_tx.is_none());
        assert!(plugin.inbound_rx.is_none());
        assert!(plugin.notice_rx.is_none());
    }

    #[tokio::test]
    async fn test_drain_connector_notices_no_rx() {
        // When notice_rx is None, drain is a no-op
        let manifest = mcp_manifest("test-no-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        assert!(plugin.notice_rx.is_none());
        // Should not panic
        plugin.drain_connector_notices();
        assert!(plugin.connectors().is_empty());
    }

    #[tokio::test]
    async fn test_drain_connector_notices_populates_shared() {
        let manifest = connector_manifest("test-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        let (notice_tx, notice_rx) =
            tokio::sync::mpsc::unbounded_channel::<astrid_mcp::ServerNotice>();
        plugin.notice_rx = Some(notice_rx);

        // Send a ConnectorsRegistered notice
        notice_tx
            .send(ServerNotice::ConnectorsRegistered {
                server_name: "plugin:test-conn".to_string(),
                channels: vec![BridgeChannelInfo {
                    name: "telegram".to_string(),
                    definition: None,
                }],
            })
            .unwrap();

        plugin.drain_connector_notices();

        assert_eq!(plugin.registered_connectors.len(), 1);
        assert_eq!(plugin.registered_connectors[0].name, "telegram");

        // shared_connectors should also be populated
        let shared = plugin.shared_connectors.lock().unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].name, "telegram");
    }

    #[tokio::test]
    async fn test_drain_connector_notices_deduplicates() {
        let manifest = connector_manifest("test-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        let (notice_tx, notice_rx) =
            tokio::sync::mpsc::unbounded_channel::<astrid_mcp::ServerNotice>();
        plugin.notice_rx = Some(notice_rx);

        // Send two notices with the same channel name
        for _ in 0..2 {
            notice_tx
                .send(ServerNotice::ConnectorsRegistered {
                    server_name: "plugin:test-conn".to_string(),
                    channels: vec![BridgeChannelInfo {
                        name: "telegram".to_string(),
                        definition: None,
                    }],
                })
                .unwrap();
        }

        plugin.drain_connector_notices();

        // Should be deduplicated by name
        assert_eq!(plugin.registered_connectors.len(), 1);
        let shared = plugin.shared_connectors.lock().unwrap();
        assert_eq!(shared.len(), 1);
    }

    #[tokio::test]
    async fn test_refresh_connectors() {
        let manifest = connector_manifest("test-conn");
        let client = test_mcp_client();
        let mut plugin = McpPlugin::new(manifest, client);

        let (notice_tx, notice_rx) =
            tokio::sync::mpsc::unbounded_channel::<astrid_mcp::ServerNotice>();
        plugin.notice_rx = Some(notice_rx);

        // Initially empty
        assert!(plugin.refresh_connectors().is_empty());

        // Send a notice
        notice_tx
            .send(ServerNotice::ConnectorsRegistered {
                server_name: "plugin:test-conn".to_string(),
                channels: vec![BridgeChannelInfo {
                    name: "discord".to_string(),
                    definition: None,
                }],
            })
            .unwrap();

        // refresh_connectors drains and returns updated list
        let connectors = plugin.refresh_connectors();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0].name, "discord");
    }
}
