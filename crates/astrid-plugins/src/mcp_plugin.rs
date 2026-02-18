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
use std::path::PathBuf;
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
    tools: Vec<Arc<dyn PluginTool>>,
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
    /// Plugin install directory — used as `current_dir` when spawning the
    /// subprocess so that relative paths in `args` resolve correctly.
    plugin_dir: Option<PathBuf>,
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
            plugin_dir: None,
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
        // Env vars that could compromise sandbox integrity. A future
        // elicitation flow will let users approve these at install time;
        // until then, reject them with a warning.
        const BLOCKED_ENV_KEYS: &[&str] = &[
            // Core execution environment
            "HOME",
            "PATH",
            // Library injection
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
            // Node.js execution control
            "NODE_OPTIONS",
            "NODE_PATH",
            // TLS/CA trust injection (MITM)
            "NODE_EXTRA_CA_CERTS",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            // Temp directory redirection
            "TMPDIR",
            "TEMP",
            "TMP",
            // Traffic interception via proxy
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
        ];

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
            if BLOCKED_ENV_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key)) {
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
            let prepared = prepare_landlock_rules(&sandbox.landlock_rules());
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
                    enforce_landlock_rules(rules).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.clone())
                    })?;

                    // Apply resource limits if configured
                    if let Some(ref limits) = rlimits {
                        apply_resource_limits(limits)?;
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

        info!(
            plugin_id = %self.id,
            server_name = %self.server_name,
            tool_count = tools.len(),
            "MCP plugin loaded successfully"
        );

        self.service = Some(service);
        self.peer = Some(peer.clone());
        self.tools = tools;
        self.state = PluginState::Ready;

        // 8. Send plugin config as a post-init notification.
        // The MCP handshake doesn't support custom initialization options,
        // so we deliver config via a custom notification. The bridge's
        // dispatch loop is already running at this point.
        if !ctx.config.is_empty() {
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

    fn tools(&self) -> &[Arc<dyn PluginTool>] {
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
fn prepare_landlock_rules(rules: &[crate::sandbox::LandlockPathRule]) -> PreparedLandlockRules {
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

    PreparedLandlockRules { rules: prepared }
}

/// Phase 2 (child process, inside `pre_exec`): create ruleset and enforce.
///
/// Only Landlock syscalls are invoked here — no heap allocation, no
/// filesystem access. All file descriptors were pre-opened in phase 1.
#[cfg(target_os = "linux")]
fn enforce_landlock_rules(prepared: PreparedLandlockRules) -> Result<(), String> {
    use landlock::{
        ABI, Access, AccessFs, CompatLevel, Compatible, PathBeneath, Ruleset, RulesetAttr,
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
        RulesetStatus::FullyEnforced
        | RulesetStatus::PartiallyEnforced
        | RulesetStatus::NotEnforced => {
            // NotEnforced: kernel doesn't support Landlock — not a fatal error
        },
    }

    Ok(())
}

/// Apply resource limits via `setrlimit` inside a `pre_exec` closure.
///
/// Uses only async-signal-safe operations: `setrlimit` is a direct syscall,
/// and `Error::last_os_error()` reads `errno` without heap allocation.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn apply_resource_limits(limits: &crate::sandbox::ResourceLimits) -> Result<(), std::io::Error> {
    // RLIMIT_NPROC — max processes/threads (per-UID, not per-process)
    let nproc = libc::rlimit {
        rlim_cur: limits.max_processes,
        rlim_max: limits.max_processes,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &raw const nproc) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // RLIMIT_AS — max virtual address space
    let address_space = libc::rlimit {
        rlim_cur: limits.max_memory_bytes,
        rlim_max: limits.max_memory_bytes,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &raw const address_space) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // RLIMIT_NOFILE — max open file descriptors
    let nofile = libc::rlimit {
        rlim_cur: limits.max_open_files,
        rlim_max: limits.max_open_files,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const nofile) } != 0 {
        return Err(std::io::Error::last_os_error());
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

    /// Verify that bad inputSchema types cause CustomResult fallthrough.
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

    /// Verify that non-string description causes CustomResult fallthrough.
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
}
