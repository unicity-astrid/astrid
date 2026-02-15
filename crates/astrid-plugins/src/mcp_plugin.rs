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

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientNotification, CustomNotification};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde_json::Value;
use tracing::{debug, error, info, warn};

use astrid_core::HookEvent;
use astrid_mcp::{AstridClientHandler, CapabilitiesHandler, McpClient, ToolResult};

use crate::context::{PluginContext, PluginToolContext};
use crate::error::{PluginError, PluginResult};
use crate::manifest::{PluginEntryPoint, PluginManifest};
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
    tools: Vec<Box<dyn PluginTool>>,
    /// MCP server name (format: `"plugin:{plugin_id}"`).
    server_name: String,
    /// Injected at construction — used for hook forwarding and lifecycle.
    mcp_client: McpClient,
    /// Running MCP service (owns the child process).
    service: Option<PluginMcpService>,
    /// Lightweight, cloneable RPC handle for tool calls + notifications.
    peer: Option<Peer<RoleClient>>,
    /// Optional sandbox profile applied to the child process.
    sandbox: Option<SandboxProfile>,
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
            service: None,
            peer: None,
            sandbox: None,
        }
    }

    /// Set an OS sandbox profile for the child process.
    #[must_use]
    pub fn with_sandbox(mut self, profile: SandboxProfile) -> Self {
        self.sandbox = Some(profile);
        self
    }

    /// Send a hook event notification to the MCP server.
    ///
    /// Sends a custom MCP notification with method
    /// `notifications/astrid.hookEvent`. This is fire-and-forget;
    /// errors are logged but do not propagate.
    pub async fn send_hook_event(&self, event: HookEvent, data: Value) {
        let Some(peer) = &self.peer else {
            debug!(
                plugin_id = %self.id,
                "Cannot send hook event: no peer connection"
            );
            return;
        };

        let notification = CustomNotification::new(
            "notifications/astrid.hookEvent",
            Some(serde_json::json!({
                "event": event.to_string(),
                "data": data,
            })),
        );

        if let Err(e) = peer
            .send_notification(ClientNotification::CustomNotification(notification))
            .await
        {
            warn!(
                plugin_id = %self.id,
                event = %event,
                error = %e,
                "Failed to send hook event to plugin MCP server"
            );
        }
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

        let alive = self.service.as_ref().is_some_and(|s| !s.is_closed());

        if !alive {
            let msg = "MCP server process exited unexpectedly".to_string();
            warn!(plugin_id = %self.id, "{msg}");
            self.state = PluginState::Failed(msg);
            self.peer = None;
            self.tools.clear();
        }

        alive
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

        for (key, value) in env {
            cmd.env(key, value);
        }

        // On Linux, apply Landlock rules via pre_exec hook.
        // PathFds are opened HERE (before fork) where heap allocation is safe.
        // Only raw Landlock syscalls run inside the pre_exec closure.
        #[cfg(target_os = "linux")]
        if let Some(sandbox) = &self.sandbox {
            let prepared = prepare_landlock_rules(&sandbox.landlock_rules())
                .map_err(|e| PluginError::SandboxError(format!("Landlock preparation: {e}")))?;
            let mut prepared = Some(prepared);
            // SAFETY: pre_exec runs between fork() and exec(). The closure
            // only invokes Landlock syscalls (landlock_create_ruleset,
            // landlock_add_rule, landlock_restrict_self) using pre-opened
            // file descriptors. No heap allocation occurs inside the closure.
            unsafe {
                cmd.pre_exec(move || {
                    let rules = prepared.take().ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "Landlock pre_exec called more than once",
                        )
                    })?;
                    enforce_landlock_rules(rules).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.to_string())
                    })
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

    async fn load(&mut self, _ctx: &PluginContext) -> PluginResult<()> {
        self.state = PluginState::Loading;

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
        let handler = Arc::new(CapabilitiesHandler::new());
        let client_handler = AstridClientHandler::new(&self.server_name, handler);

        let service = client_handler.serve(transport).await.map_err(|e| {
            let err = PluginError::McpServerFailed {
                plugin_id: self.id.clone(),
                message: format!("MCP handshake failed: {e}"),
            };
            self.state = PluginState::Failed(err.to_string());
            err
        })?;

        // 5. Discover tools
        let rmcp_tools = service.list_all_tools().await.map_err(|e| {
            let err = PluginError::McpServerFailed {
                plugin_id: self.id.clone(),
                message: format!("Failed to list tools: {e}"),
            };
            self.state = PluginState::Failed(err.to_string());
            err
        })?;

        // 6. Extract peer handle
        let peer = service.peer().clone();

        // 7. Create McpPluginTool wrappers
        let tools: Vec<Box<dyn PluginTool>> = rmcp_tools
            .iter()
            .map(|t| {
                let tool: Box<dyn PluginTool> = Box::new(McpPluginTool {
                    name: t.name.to_string(),
                    description: t.description.as_deref().unwrap_or("").to_string(),
                    input_schema: serde_json::to_value(&*t.input_schema)
                        .unwrap_or_else(|_| serde_json::json!({"type": "object"})),
                    server_name: self.server_name.clone(),
                    peer: peer.clone(),
                });
                tool
            })
            .collect();

        info!(
            plugin_id = %self.id,
            server_name = %self.server_name,
            tool_count = tools.len(),
            "MCP plugin loaded successfully"
        );

        self.service = Some(service);
        self.peer = Some(peer);
        self.tools = tools;
        self.state = PluginState::Ready;

        Ok(())
    }

    async fn unload(&mut self) -> PluginResult<()> {
        self.state = PluginState::Unloading;

        // Drop the peer handle first
        self.peer = None;
        self.tools.clear();

        // Gracefully close the MCP session
        if let Some(ref mut service) = self.service {
            match service.close_with_timeout(SHUTDOWN_TIMEOUT).await {
                Ok(Some(reason)) => {
                    info!(
                        plugin_id = %self.id,
                        ?reason,
                        "Plugin MCP session closed gracefully"
                    );
                },
                Ok(None) => {
                    warn!(
                        plugin_id = %self.id,
                        "Plugin MCP session close timed out; dropping"
                    );
                },
                Err(e) => {
                    error!(
                        plugin_id = %self.id,
                        error = %e,
                        "Plugin MCP session close join error"
                    );
                },
            }
        }

        self.service = None;
        self.state = PluginState::Unloaded;

        info!(plugin_id = %self.id, "MCP plugin unloaded");

        Ok(())
    }

    fn tools(&self) -> &[Box<dyn PluginTool>] {
        &self.tools
    }
}

/// A tool provided by an MCP server, wrapped as a [`PluginTool`].
///
/// Tool calls are forwarded directly to the MCP server via the stored
/// [`Peer`] handle. Security is enforced at the runtime layer (before
/// `execute()` is called), not here.
struct McpPluginTool {
    name: String,
    description: String,
    input_schema: Value,
    #[allow(dead_code)]
    server_name: String,
    peer: Peer<RoleClient>,
}

#[async_trait]
impl PluginTool for McpPluginTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    async fn execute(&self, args: Value, _ctx: &PluginToolContext) -> PluginResult<String> {
        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), other);
                Some(map)
            },
        };

        let params = CallToolRequestParams {
            meta: None,
            name: Cow::Owned(self.name.clone()),
            arguments,
            task: None,
        };

        let result = self
            .peer
            .call_tool(params)
            .await
            .map_err(|e| PluginError::ExecutionFailed(format!("MCP tool call failed: {e}")))?;

        // Convert to our ToolResult and extract text content
        let tool_result = ToolResult::from(result);
        if tool_result.is_error {
            return Err(PluginError::ExecutionFailed(
                tool_result
                    .error
                    .unwrap_or_else(|| "Unknown MCP tool error".into()),
            ));
        }

        Ok(tool_result.text_content())
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
) -> PluginResult<Box<dyn Plugin>> {
    match &manifest.entry_point {
        PluginEntryPoint::Wasm { .. } => Err(PluginError::UnsupportedEntryPoint("wasm".into())),
        PluginEntryPoint::Mcp { .. } => {
            let client = mcp_client.ok_or(PluginError::McpClientRequired)?;
            Ok(Box::new(McpPlugin::new(manifest, client)))
        },
    }
}

/// A pre-opened Landlock rule ready for enforcement inside `pre_exec`.
///
/// File descriptors are opened in the parent process (where allocation is
/// safe) and consumed inside the `pre_exec` closure (where only
/// async-signal-safe operations are permitted).
#[cfg(target_os = "linux")]
struct PreparedLandlockRules {
    /// Pre-opened `(PathFd, read, write)` tuples.
    rules: Vec<(landlock::PathFd, bool, bool)>,
}

/// Phase 1 (parent process): open file descriptors and compute access flags.
///
/// This runs before `fork()`, so heap allocation and filesystem access are
/// safe. Paths that don't exist are silently skipped.
#[cfg(target_os = "linux")]
fn prepare_landlock_rules(
    rules: &[crate::sandbox::LandlockPathRule],
) -> Result<PreparedLandlockRules, String> {
    use landlock::PathFd;

    let mut prepared = Vec::with_capacity(rules.len());

    for rule in rules {
        if !rule.read && !rule.write {
            continue;
        }

        // Open the path FD now (heap allocation happens here, safely)
        if let Ok(fd) = PathFd::new(&rule.path) {
            prepared.push((fd, rule.read, rule.write));
        }
    }

    Ok(PreparedLandlockRules { rules: prepared })
}

/// Phase 2 (child process, inside `pre_exec`): create ruleset and enforce.
///
/// Only Landlock syscalls are invoked here — no heap allocation, no
/// filesystem access. All file descriptors were pre-opened in phase 1.
#[cfg(target_os = "linux")]
fn enforce_landlock_rules(prepared: PreparedLandlockRules) -> Result<(), String> {
    use landlock::{
        ABI, AccessFs, CompatLevel, Compatible, PathBeneath, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetStatus,
    };

    let abi = ABI::V5;

    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| format!("failed to create Landlock ruleset: {e}"))?
        .create()
        .map_err(|e| format!("failed to create Landlock ruleset: {e}"))?;

    for (fd, read, write) in prepared.rules {
        let access = match (read, write) {
            (true, true) => AccessFs::from_all(abi),
            (true, false) => AccessFs::from_read(abi),
            (false, true) => AccessFs::from_write(abi),
            (false, false) => continue,
        };
        let path_beneath = PathBeneath::new(fd, access);
        ruleset = ruleset
            .add_rule(path_beneath)
            .map_err(|e| format!("failed to add Landlock rule: {e}"))?;
    }

    let status = ruleset
        .restrict_self()
        .map_err(|e| format!("failed to enforce Landlock ruleset: {e}"))?;

    match status.ruleset {
        RulesetStatus::FullyEnforced | RulesetStatus::PartiallyEnforced => {},
        RulesetStatus::NotEnforced => {
            // Kernel doesn't support Landlock — not a fatal error
        },
    }

    Ok(())
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
            config: HashMap::new(),
        }
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
        let plugin = create_plugin(manifest, Some(client));
        assert!(plugin.is_ok());
    }

    #[test]
    fn test_create_plugin_mcp_requires_client() {
        let manifest = mcp_manifest("test-mcp");
        let result = create_plugin(manifest, None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::McpClientRequired
        ));
    }

    #[test]
    fn test_create_plugin_wasm_unsupported() {
        let manifest = wasm_manifest("test-wasm");
        let result = create_plugin(manifest, None);
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
}
