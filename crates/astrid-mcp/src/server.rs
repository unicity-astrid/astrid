//! MCP server lifecycle management.
//!
//! Handles starting, stopping, and managing MCP server processes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use astrid_core::retry::RetryConfig;

use rmcp::ServiceExt;
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;

use crate::capabilities::CapabilitiesHandler;
use crate::capabilities::{AstridClientHandler, ServerNotice};
use crate::config::{RestartPolicy, ServerConfig, ServersConfig, Transport};
use crate::error::{McpError, McpResult};
use crate::types::{ServerInfo, ToolDefinition};

use tokio::sync::mpsc;

/// Type alias for a running MCP client service.
type McpService = RunningService<RoleClient, AstridClientHandler>;

/// A running MCP server instance.
pub(crate) struct RunningServer {
    /// Server configuration.
    pub config: ServerConfig,
    /// Running rmcp service (handles child process lifecycle).
    service: Option<McpService>,
    /// Server info after initialization.
    pub info: Option<ServerInfo>,
    /// Available tools.
    pub tools: Vec<ToolDefinition>,
    /// Whether the server is connected and ready.
    pub ready: bool,
    /// How many times this server has been restarted.
    pub restart_count: u32,
    /// When the last restart attempt was made (for backoff calculations).
    pub last_restart_attempt: Option<Instant>,
}

impl RunningServer {
    /// Create a new (not-yet-connected) running server.
    fn new(config: ServerConfig) -> Self {
        Self {
            config,
            service: None,
            info: None,
            tools: Vec::new(),
            ready: false,
            restart_count: 0,
            last_restart_attempt: None,
        }
    }

    /// Check if the server is still connected.
    pub(crate) fn is_alive(&self) -> bool {
        match &self.service {
            Some(svc) => !svc.is_closed(),
            None => false,
        }
    }

    /// Get a cloneable peer handle for making requests.
    pub(crate) fn peer(&self) -> Option<Peer<RoleClient>> {
        self.service.as_ref().map(|svc| svc.peer().clone())
    }
}

/// Build a `tokio::process::Command` for a trusted (unsandboxed) server.
fn build_unsandboxed_command(
    name: &str,
    command: &str,
    config: &ServerConfig,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(&config.args);

    for (key, value) in &config.env {
        if astrid_core::env_policy::is_blocked_spawn_env(key) {
            warn!(
                server = %name,
                key = %key,
                "Ignoring blocked env var from server config"
            );
            continue;
        }
        cmd.env(key, value);
    }

    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }

    info!(server = name, "Spawning trusted (unsandboxed) MCP server");
    cmd
}

/// Manages MCP server lifecycles.
pub struct ServerManager {
    /// Server configurations.
    configs: ServersConfig,
    /// Running servers.
    running: Arc<RwLock<HashMap<String, RunningServer>>>,
    /// Timeout for graceful `close_with_timeout` during shutdown.
    shutdown_timeout: std::time::Duration,
    /// Workspace root for sandbox writable directory.
    ///
    /// When `None`, sandboxing falls back to `config.cwd` or a temp directory.
    workspace_root: Option<PathBuf>,
}

impl ServerManager {
    /// Create a new server manager.
    #[must_use]
    pub fn new(configs: ServersConfig) -> Self {
        let shutdown_timeout = configs.shutdown_timeout;
        Self {
            configs,
            running: Arc::new(RwLock::new(HashMap::new())),
            shutdown_timeout,
            workspace_root: None,
        }
    }

    /// Set the workspace root directory for sandbox writable access.
    ///
    /// When sandboxing is active (`trusted: false`), the sandboxed process
    /// will have write access to this directory. If not set, falls back
    /// to the server's `cwd` or a system temp directory.
    #[must_use]
    pub fn with_workspace_root(mut self, root: PathBuf) -> Self {
        self.workspace_root = Some(root);
        self
    }

    /// Create from default configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be loaded.
    pub fn from_default_config() -> McpResult<Self> {
        let configs = ServersConfig::load_default()?;
        Ok(Self::new(configs))
    }

    /// Get server configuration by name.
    #[must_use]
    pub fn get_config(&self, name: &str) -> Option<&ServerConfig> {
        self.configs.get(name)
    }

    /// List all configured servers.
    #[must_use]
    pub fn list_configured(&self) -> Vec<&str> {
        self.configs.list()
    }

    /// List running servers.
    pub async fn list_running(&self) -> Vec<String> {
        let running = self.running.read().await;
        running.keys().cloned().collect()
    }

    /// Check if a server is running.
    pub async fn is_running(&self, name: &str) -> bool {
        let running = self.running.read().await;
        running.contains_key(name)
    }

    /// Register a server in the running map (validates config, verifies binary).
    ///
    /// This does NOT establish the MCP connection; call `connect_server` for that.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The server is already running
    /// - The server configuration is not found
    /// - Binary verification fails
    pub async fn start(&self, name: &str) -> McpResult<()> {
        // Check if already running
        {
            let running = self.running.read().await;
            if running.contains_key(name) {
                return Err(McpError::ServerAlreadyRunning {
                    name: name.to_string(),
                });
            }
        }

        // Get configuration
        let config = self
            .configs
            .get(name)
            .ok_or_else(|| McpError::ServerNotFound {
                name: name.to_string(),
            })?
            .clone();

        info!(server = name, "Registering MCP server");

        // Verify binary hash if configured
        if let Err(e) = config.verify_binary() {
            error!(server = name, error = %e, "Binary verification failed");
            return Err(e);
        }

        // Store in running map (not yet connected)
        {
            let mut running = self.running.write().await;
            running.insert(name.to_string(), RunningServer::new(config));
        }

        Ok(())
    }

    /// Add a server configuration dynamically and register it.
    ///
    /// # Errors
    /// Returns an error if the server is already running.
    pub async fn add_server(&self, name: &str, config: ServerConfig) -> McpResult<()> {
        let mut running = self.running.write().await;
        if running.contains_key(name) {
            return Err(McpError::ServerAlreadyRunning {
                name: name.to_string(),
            });
        }

        info!(server = name, "Dynamically registering MCP server");

        if let Err(e) = config.verify_binary() {
            error!(server = name, error = %e, "Binary verification failed");
            return Err(e);
        }

        running.insert(name.to_string(), RunningServer::new(config));
        Ok(())
    }

    /// Establish the actual MCP connection for a registered server.
    ///
    /// Spawns the child process, performs the MCP handshake, and fetches
    /// the tool list from the server.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The server is not registered
    /// - The transport cannot be created
    /// - The MCP handshake fails
    pub(crate) async fn connect_server(
        &self,
        name: &str,
        handler: Arc<CapabilitiesHandler>,
        notice_tx: Option<mpsc::UnboundedSender<ServerNotice>>,
    ) -> McpResult<()> {
        let config = {
            let running = self.running.read().await;
            let server = running
                .get(name)
                .ok_or_else(|| McpError::ServerNotRunning {
                    name: name.to_string(),
                })?;
            server.config.clone()
        };

        match config.transport {
            Transport::Stdio => {
                self.connect_stdio_server(name, &config, handler, notice_tx)
                    .await?;
            },
            Transport::Sse => {
                return Err(McpError::ConfigError(
                    "SSE transport not yet supported; enable `transport-streamable-http-client` \
                     feature in rmcp"
                        .to_string(),
                ));
            },
        }

        Ok(())
    }

    /// Connect to a stdio server via `TokioChildProcess`.
    async fn connect_stdio_server(
        &self,
        name: &str,
        config: &ServerConfig,
        handler: Arc<CapabilitiesHandler>,
        notice_tx: Option<mpsc::UnboundedSender<ServerNotice>>,
    ) -> McpResult<()> {
        let command = config.command.as_ref().ok_or_else(|| {
            McpError::ConfigError(format!("No command specified for stdio server {name}"))
        })?;

        let cmd = if config.trusted {
            build_unsandboxed_command(name, command, config)
        } else {
            self.build_sandboxed_command(name, command, config)?
        };

        // Create transport (spawns the child process)
        let transport = TokioChildProcess::new(cmd).map_err(|e| McpError::ServerStartFailed {
            name: name.to_string(),
            reason: e.to_string(),
        })?;

        // Create the client handler and perform the MCP handshake
        let mut client_handler = AstridClientHandler::new(name, handler);
        if let Some(tx) = notice_tx {
            client_handler = client_handler.with_notice_tx(tx);
        }
        let service = client_handler.serve(transport).await.map_err(|e| {
            McpError::InitializationFailed(format!("MCP handshake failed for {name}: {e}"))
        })?;

        // Get server info from the handshake result
        let server_info = service
            .peer_info()
            .map(|info| ServerInfo::from_rmcp(info, name));

        // Fetch available tools
        let rmcp_tools = service.list_all_tools().await.map_err(McpError::from)?;
        let tools: Vec<ToolDefinition> = rmcp_tools
            .iter()
            .map(|t| ToolDefinition::from_rmcp(t, name))
            .collect();

        info!(
            server = name,
            tool_count = tools.len(),
            "MCP connection established"
        );

        // Store everything
        {
            let mut running = self.running.write().await;
            if let Some(server) = running.get_mut(name) {
                server.service = Some(service);
                server.info = server_info;
                server.tools = tools;
                server.ready = true;
            }
        }

        Ok(())
    }

    /// Build a sandboxed `tokio::process::Command` for an untrusted server.
    ///
    /// Applies OS-level sandboxing (bwrap on Linux, sandbox-exec on macOS),
    /// scrubs inherited environment variables, and hides `~/.astrid/`.
    fn build_sandboxed_command(
        &self,
        name: &str,
        command: &str,
        config: &ServerConfig,
    ) -> McpResult<tokio::process::Command> {
        use astrid_workspace::ProcessSandboxConfig;

        // config.cwd doubles as both the sandbox writable root and the process CWD.
        // When set, the sandboxed process can write to its own working directory.
        // Fallback order: config.cwd > workspace_root > temp_dir/astrid-mcp/<name>
        let writable_root = config
            .cwd
            .clone()
            .or_else(|| self.workspace_root.clone())
            .unwrap_or_else(|| std::env::temp_dir().join("astrid-mcp").join(name));

        // Ensure the writable root exists before bwrap tries to bind-mount it.
        std::fs::create_dir_all(&writable_root).map_err(|e| McpError::ServerStartFailed {
            name: name.to_string(),
            reason: format!(
                "Failed to create writable root {}: {e}",
                writable_root.display()
            ),
        })?;

        // Resolve ~/.astrid/ path - this is mandatory for untrusted servers.
        let astrid_home = Self::resolve_astrid_home()?;

        // Build sandbox config
        let mut sandbox_config = ProcessSandboxConfig::new(&writable_root)
            .with_network(config.allow_network)
            .with_hidden(astrid_home);

        // Add config-specified extra paths. Validated for:
        // 1. Absolute (avoid ambiguity about which directory they resolve relative to)
        // 2. No double-quotes (prevent SBPL profile injection on macOS)
        for path in &config.allowed_read_paths {
            Self::validate_sandbox_path(path, "allowed_read_paths")?;
            sandbox_config = sandbox_config.with_extra_read(path);
        }
        for path in &config.allowed_write_paths {
            Self::validate_sandbox_path(path, "allowed_write_paths")?;
            sandbox_config = sandbox_config.with_extra_write(path);
        }

        // Add common package manager cache dirs as read-only so npm/cargo
        // don't re-download on every server start.
        if let Ok(home) = std::env::var("HOME") {
            for cache_dir in &[".npm", ".nvm", ".cargo", ".rustup"] {
                let cache_path = std::path::PathBuf::from(&home).join(cache_dir);
                if cache_path.exists() {
                    sandbox_config = sandbox_config.with_extra_read(cache_path);
                }
            }
        }

        // Get sandbox prefix (bwrap/sandbox-exec args)
        let sandbox_prefix = sandbox_config.sandbox_prefix();

        // Build the command
        let mut cmd = if let Some(prefix) = sandbox_prefix {
            let mut cmd = tokio::process::Command::new(&prefix.program);
            for arg in &prefix.args {
                cmd.arg(arg);
            }
            cmd.arg(command);
            cmd.args(&config.args);
            cmd
        } else {
            warn!(
                server = name,
                "Sandboxing not available on this platform; \
                 untrusted MCP server will run without OS-level isolation"
            );
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(&config.args);
            cmd
        };

        // Environment scrubbing: clear inherited env, re-add safe vars.
        cmd.env_clear();

        // Use a fixed PATH so the parent process can't influence binary
        // resolution inside the sandbox. HOME/USER/SHELL/TERM/LANG are
        // identity/locale vars that are safe to forward from the parent.
        cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
        for var in &["HOME", "USER", "SHELL", "TERM", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        // Apply config env vars (filtered through blocklist)
        for (key, value) in &config.env {
            if astrid_core::env_policy::is_blocked_spawn_env(key) {
                warn!(
                    server = %name,
                    key = %key,
                    "Ignoring blocked env var from server config"
                );
                continue;
            }
            cmd.env(key, value);
        }

        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }

        info!(
            server = name,
            writable_root = %writable_root.display(),
            allow_network = config.allow_network,
            "Spawning sandboxed MCP server"
        );

        Ok(cmd)
    }

    /// Validate a path for use in sandbox configuration.
    ///
    /// Rejects relative paths and paths containing double-quote characters
    /// (which would break macOS Seatbelt SBPL profile syntax).
    fn validate_sandbox_path(path: &std::path::Path, field: &str) -> McpResult<()> {
        if !path.is_absolute() {
            return Err(McpError::ConfigError(format!(
                "{field} must be absolute, got: {}",
                path.display()
            )));
        }
        if path.to_string_lossy().contains('"') {
            return Err(McpError::ConfigError(format!(
                "{field} must not contain double-quote characters, got: {}",
                path.display()
            )));
        }
        Ok(())
    }

    /// Resolve the `~/.astrid/` directory path.
    ///
    /// This is mandatory for untrusted servers - if we can't determine
    /// the path, we refuse to start the server rather than running it
    /// with `~/.astrid/` exposed.
    fn resolve_astrid_home() -> McpResult<std::path::PathBuf> {
        // Try AstridHome::resolve() first
        if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            return Ok(home.root().to_path_buf());
        }

        // Fallback: construct from $HOME
        if let Ok(home) = std::env::var("HOME") {
            return Ok(std::path::PathBuf::from(home).join(".astrid"));
        }

        Err(McpError::ServerStartFailed {
            name: "sandbox".to_string(),
            reason: "Cannot determine ~/.astrid/ path for sandbox hiding. \
                     Set $HOME or $ASTRID_HOME, or mark the server as trusted."
                .to_string(),
        })
    }

    /// Get a cloneable peer handle for a running server.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not running or not connected.
    pub async fn get_peer(&self, name: &str) -> McpResult<Peer<RoleClient>> {
        let running = self.running.read().await;
        let server = running
            .get(name)
            .ok_or_else(|| McpError::ServerNotRunning {
                name: name.to_string(),
            })?;

        server.peer().ok_or_else(|| {
            McpError::ConnectionFailed(format!("Server {name} is registered but not connected"))
        })
    }

    /// Stop a server.
    ///
    /// Performs a graceful shutdown via `close_with_timeout` on the MCP
    /// session before dropping the `RunningServer`.  This avoids the rmcp
    /// `Drop` warning about asynchronous cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not running.
    pub async fn stop(&self, name: &str) -> McpResult<()> {
        let mut running = self.running.write().await;

        let mut server = running
            .remove(name)
            .ok_or_else(|| McpError::ServerNotRunning {
                name: name.to_string(),
            })?;

        info!(server = name, "Stopping MCP server");

        // Gracefully close the MCP session before dropping.
        if let Some(ref mut service) = server.service {
            match service.close_with_timeout(self.shutdown_timeout).await {
                Ok(Some(reason)) => {
                    info!(server = name, ?reason, "MCP session closed gracefully");
                },
                Ok(None) => {
                    warn!(
                        server = name,
                        timeout_secs = self.shutdown_timeout.as_secs(),
                        "MCP session close timed out; dropping"
                    );
                },
                Err(e) => {
                    warn!(server = name, error = %e, "MCP session close join error");
                },
            }
        }

        drop(server);

        Ok(())
    }

    /// Restart a server: stop → start → connect.
    ///
    /// Increments the restart counter for the server.
    ///
    /// # Errors
    ///
    /// Returns an error if the server fails to start or connect.
    pub(crate) async fn restart(
        &self,
        name: &str,
        handler: Arc<CapabilitiesHandler>,
        notice_tx: Option<mpsc::UnboundedSender<ServerNotice>>,
    ) -> McpResult<()> {
        // Remember the previous restart count.
        let prev_count = {
            let running = self.running.read().await;
            running.get(name).map_or(0, |s| s.restart_count)
        };

        // Stop if running.
        if self.is_running(name).await {
            self.stop(name).await?;
        }

        // Register + connect.
        self.start(name).await?;
        self.connect_server(name, handler, notice_tx).await?;

        // Restore and increment restart count.
        let new_count = prev_count.saturating_add(1);
        {
            let mut running = self.running.write().await;
            if let Some(server) = running.get_mut(name) {
                server.restart_count = new_count;
            }
        }

        info!(server = name, restart_count = new_count, "Server restarted");
        Ok(())
    }

    /// Stop all servers.
    ///
    /// # Errors
    ///
    /// Returns `Ok(())` even if individual servers fail to stop (warnings are logged).
    pub async fn stop_all(&self) -> McpResult<()> {
        let names: Vec<String> = {
            let running = self.running.read().await;
            running.keys().cloned().collect()
        };

        for name in names {
            if let Err(e) = self.stop(&name).await {
                warn!(server = name, error = %e, "Failed to stop server");
            }
        }

        Ok(())
    }

    /// Start all auto-start servers.
    ///
    /// # Errors
    ///
    /// Returns `Ok(())` even if individual servers fail to start (warnings are logged).
    pub async fn start_auto_servers(&self) -> McpResult<()> {
        let auto_servers = self.configs.auto_start_servers();

        for config in auto_servers {
            if let Err(e) = self.start(&config.name).await {
                warn!(
                    server = config.name,
                    error = %e,
                    "Failed to auto-start server"
                );
            }
        }

        Ok(())
    }

    /// Update server tools after connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not running.
    pub async fn set_server_tools(&self, name: &str, tools: Vec<ToolDefinition>) -> McpResult<()> {
        let mut running = self.running.write().await;
        let server = running
            .get_mut(name)
            .ok_or_else(|| McpError::ServerNotRunning {
                name: name.to_string(),
            })?;

        server.tools = tools;

        Ok(())
    }

    /// Get all tools from all running servers.
    pub async fn all_tools(&self) -> Vec<ToolDefinition> {
        let running = self.running.read().await;
        running.values().flat_map(|s| s.tools.clone()).collect()
    }

    /// Check health of all running servers.
    pub async fn health_check(&self) -> HashMap<String, bool> {
        let running = self.running.read().await;
        let mut health = HashMap::new();

        for (name, server) in running.iter() {
            health.insert(name.clone(), server.is_alive());
        }

        health
    }

    /// Backoff configuration for restart attempts.
    ///
    /// Uses 30 s base delay, 5 min cap, exponential base 2.
    fn restart_backoff() -> RetryConfig {
        RetryConfig::new(
            u32::MAX, // max_attempts handled by RestartPolicy, not RetryConfig
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(300),
            2.0,
        )
    }

    /// Check whether a dead server should be restarted based on its `RestartPolicy`.
    ///
    /// Also accounts for backoff cooldown — if the cooldown period for the
    /// current restart count has not elapsed, returns `false`.
    ///
    /// **Note:** This is a read-only query. For actual restarts, prefer
    /// [`restart_if_allowed`] which atomically checks the policy and
    /// performs the restart, avoiding TOCTOU races on `restart_count`.
    pub async fn should_restart(&self, name: &str) -> bool {
        let Some(config) = self.configs.get(name) else {
            return false;
        };

        let (restart_count, last_attempt) = {
            let running = self.running.read().await;
            running
                .get(name)
                .map_or((0, None), |s| (s.restart_count, s.last_restart_attempt))
        };

        let allowed = match &config.restart_policy {
            RestartPolicy::Never => false,
            RestartPolicy::OnFailure { max_retries } => restart_count < *max_retries,
            RestartPolicy::Always => true,
        };

        if !allowed {
            return false;
        }

        // Check backoff cooldown.
        if let Some(last) = last_attempt {
            let backoff = Self::restart_backoff();
            // restart_count is 0-indexed for attempts that already happened,
            // but delay_for_attempt(0) = ZERO, so use restart_count directly
            // (it represents the next attempt number).
            let required_delay = backoff.delay_for_attempt(restart_count);
            if last.elapsed() < required_delay {
                return false;
            }
        }

        true
    }

    /// Atomically check the restart policy and restart if allowed.
    ///
    /// Holds the write lock during the policy check *and* server removal,
    /// so concurrent callers cannot both pass the retry-limit check.
    /// The lock is released before I/O (process spawn, MCP handshake).
    ///
    /// Returns `Ok(true)` if the server was restarted, `Ok(false)` if the
    /// policy forbids it.
    ///
    /// # Errors
    ///
    /// Returns an error if the restart itself fails (start or connect).
    pub(crate) async fn restart_if_allowed(
        &self,
        name: &str,
        handler: Arc<CapabilitiesHandler>,
        notice_tx: Option<mpsc::UnboundedSender<ServerNotice>>,
    ) -> McpResult<bool> {
        let Some(config) = self.configs.get(name) else {
            return Ok(false);
        };

        let backoff = Self::restart_backoff();

        // Atomic: check policy + backoff + remove server under a single write lock.
        let prev_count = {
            let mut running = self.running.write().await;
            let (count, last_attempt) = running
                .get(name)
                .map_or((0, None), |s| (s.restart_count, s.last_restart_attempt));

            let allowed = match &config.restart_policy {
                RestartPolicy::Never => false,
                RestartPolicy::OnFailure { max_retries } => count < *max_retries,
                RestartPolicy::Always => true,
            };

            if !allowed {
                return Ok(false);
            }

            // Check backoff cooldown: if the required delay has not elapsed
            // since the last restart attempt, skip this restart.
            if let Some(last) = last_attempt {
                let required_delay = backoff.delay_for_attempt(count);
                if last.elapsed() < required_delay {
                    return Ok(false);
                }
            }

            // Remove while holding the write lock — prevents concurrent
            // callers from also passing the policy check for the same server.
            running.remove(name);
            count
        };
        // Write lock released. The server entry is gone, so any concurrent
        // caller will see restart_count = 0 (map_or default) but the server
        // is absent, and `start()` will re-register it fresh.

        // Re-register (validates config, verifies binary hash).
        self.start(name).await?;

        // Establish MCP connection (process spawn + handshake).
        if let Err(e) = self.connect_server(name, handler, notice_tx).await {
            // Clean up the registered-but-not-connected entry.
            let _ = self.stop(name).await;
            return Err(e);
        }

        // Set the incremented restart count and record the attempt timestamp.
        let new_count = prev_count.saturating_add(1);
        {
            let mut running = self.running.write().await;
            if let Some(server) = running.get_mut(name) {
                server.restart_count = new_count;
                server.last_restart_attempt = Some(Instant::now());
            }
        }

        info!(
            server = name,
            restart_count = new_count,
            "Server restarted (policy-allowed)"
        );
        Ok(true)
    }

    /// List names of servers configured for auto-start.
    #[must_use]
    pub fn list_auto_start_names(&self) -> Vec<String> {
        self.configs
            .auto_start_servers()
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    /// Number of running servers.
    pub async fn running_count(&self) -> usize {
        self.running.read().await.len()
    }

    /// Number of configured servers.
    #[must_use]
    pub fn configured_count(&self) -> usize {
        self.configs.servers.len()
    }
}

impl std::fmt::Debug for ServerManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerManager")
            .field("configured_servers", &self.configs.list())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_manager_creation() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs);

        assert!(manager.list_configured().is_empty());
        assert!(manager.list_running().await.is_empty());
    }

    #[tokio::test]
    async fn test_server_not_found() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs);

        let result = manager.start("nonexistent").await;
        assert!(matches!(result, Err(McpError::ServerNotFound { .. })));
    }

    #[tokio::test]
    async fn test_is_running() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs);

        assert!(!manager.is_running("test").await);
    }

    #[test]
    fn restart_backoff_delays_are_exponential() {
        let backoff = ServerManager::restart_backoff();

        // attempt 0 = no delay (initial attempt).
        assert_eq!(backoff.delay_for_attempt(0), std::time::Duration::ZERO);
        // attempt 1 = 30 s base.
        assert_eq!(
            backoff.delay_for_attempt(1),
            std::time::Duration::from_secs(30)
        );
        // attempt 2 = 30 * 2 = 60 s.
        assert_eq!(
            backoff.delay_for_attempt(2),
            std::time::Duration::from_secs(60)
        );
        // attempt 3 = 30 * 4 = 120 s.
        assert_eq!(
            backoff.delay_for_attempt(3),
            std::time::Duration::from_secs(120)
        );
        // attempt 4 = 30 * 8 = 240 s.
        assert_eq!(
            backoff.delay_for_attempt(4),
            std::time::Duration::from_secs(240)
        );
        // attempt 5 = 30 * 16 = 480 s, capped at 300 s.
        assert_eq!(
            backoff.delay_for_attempt(5),
            std::time::Duration::from_secs(300)
        );
        // further attempts also capped at 300 s.
        assert_eq!(
            backoff.delay_for_attempt(10),
            std::time::Duration::from_secs(300)
        );
    }

    #[tokio::test]
    async fn should_restart_never_policy() {
        let mut configs = ServersConfig::default();
        configs.add(ServerConfig::stdio("srv", "cmd").with_restart_policy(RestartPolicy::Never));
        let manager = ServerManager::new(configs);

        assert!(!manager.should_restart("srv").await);
    }

    #[tokio::test]
    async fn should_restart_always_policy_no_running_entry() {
        let mut configs = ServersConfig::default();
        configs.add(ServerConfig::stdio("srv", "cmd").with_restart_policy(RestartPolicy::Always));
        let manager = ServerManager::new(configs);

        // No running entry and no last_restart_attempt → should allow.
        assert!(manager.should_restart("srv").await);
    }

    #[tokio::test]
    async fn should_restart_respects_backoff_cooldown() {
        let mut configs = ServersConfig::default();
        configs.add(ServerConfig::stdio("srv", "cmd").with_restart_policy(RestartPolicy::Always));
        let manager = ServerManager::new(configs);

        // Manually insert a running server with a very recent last_restart_attempt
        // and restart_count = 1 (so delay_for_attempt(1) = 30 s).
        {
            let mut running = manager.running.write().await;
            let mut server = RunningServer::new(
                ServerConfig::stdio("srv", "cmd").with_restart_policy(RestartPolicy::Always),
            );
            server.restart_count = 1;
            server.last_restart_attempt = Some(Instant::now());
            running.insert("srv".to_string(), server);
        }

        // Cooldown not elapsed → should_restart returns false.
        assert!(!manager.should_restart("srv").await);
    }

    #[tokio::test]
    async fn should_restart_allows_after_cooldown_elapsed() {
        let mut configs = ServersConfig::default();
        configs.add(ServerConfig::stdio("srv", "cmd").with_restart_policy(RestartPolicy::Always));
        let manager = ServerManager::new(configs);

        // Insert with a restart attempt far in the past.
        {
            let mut running = manager.running.write().await;
            let mut server = RunningServer::new(
                ServerConfig::stdio("srv", "cmd").with_restart_policy(RestartPolicy::Always),
            );
            server.restart_count = 1;
            // 60 seconds ago — the required delay for attempt 1 is 30s, so this
            // is well past the cooldown.
            server.last_restart_attempt = Some(
                Instant::now()
                    .checked_sub(std::time::Duration::from_secs(60))
                    .expect("failed to sub 60s from Instant"),
            );
            running.insert("srv".to_string(), server);
        }

        assert!(manager.should_restart("srv").await);
    }

    #[tokio::test]
    async fn should_restart_on_failure_respects_max_retries() {
        let mut configs = ServersConfig::default();
        configs.add(
            ServerConfig::stdio("srv", "cmd")
                .with_restart_policy(RestartPolicy::OnFailure { max_retries: 2 }),
        );
        let manager = ServerManager::new(configs);

        // Insert with restart_count = 2 (already hit the limit).
        {
            let mut running = manager.running.write().await;
            let mut server = RunningServer::new(
                ServerConfig::stdio("srv", "cmd")
                    .with_restart_policy(RestartPolicy::OnFailure { max_retries: 2 }),
            );
            server.restart_count = 2;
            running.insert("srv".to_string(), server);
        }

        assert!(!manager.should_restart("srv").await);
    }

    #[test]
    fn test_build_unsandboxed_command() {
        let config = ServerConfig::stdio("test", "echo")
            .with_args(["hello"])
            .with_env("FOO", "bar");

        let cmd = build_unsandboxed_command("test", "echo", &config);

        // Command program should be the original command
        let program = cmd.as_std().get_program().to_string_lossy().to_string();
        assert_eq!(program, "echo");
    }

    #[test]
    fn test_build_sandboxed_command_adds_sandbox_prefix() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs)
            .with_workspace_root(std::env::temp_dir().join("astrid-test-workspace"));

        let config = ServerConfig::stdio("test", "echo").with_args(["hello"]);

        let cmd = manager
            .build_sandboxed_command("test", "echo", &config)
            .expect("should build sandboxed command");

        let program = cmd.as_std().get_program().to_string_lossy().to_string();

        // On supported platforms, the program should be the sandbox wrapper
        #[cfg(target_os = "linux")]
        assert_eq!(program, "bwrap");
        #[cfg(target_os = "macos")]
        assert_eq!(program, "sandbox-exec");
        // On unsupported platforms, falls through to original command
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        assert_eq!(program, "echo");
    }

    #[test]
    fn test_trusted_server_bypasses_sandbox() {
        let config = ServerConfig::stdio("test", "echo").trusted();

        let cmd = build_unsandboxed_command("test", "echo", &config);

        let program = cmd.as_std().get_program().to_string_lossy().to_string();
        assert_eq!(
            program, "echo",
            "trusted server should run without sandbox wrapper"
        );
    }

    #[test]
    fn test_sandboxed_command_clears_env() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs)
            .with_workspace_root(std::env::temp_dir().join("astrid-test-workspace"));

        let config = ServerConfig::stdio("test", "echo").with_env("SAFE_VAR", "value");

        let cmd = manager
            .build_sandboxed_command("test", "echo", &config)
            .expect("should build command");

        let envs: Vec<_> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|v| {
                    (
                        k.to_string_lossy().to_string(),
                        v.to_string_lossy().to_string(),
                    )
                })
            })
            .collect();

        // Config env vars should be passed through
        let has_safe_var = envs.iter().any(|(k, v)| k == "SAFE_VAR" && v == "value");
        assert!(has_safe_var, "config env vars should be passed through");

        // PATH should be the fixed value, not inherited from parent
        let path_val = envs
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str());
        assert_eq!(
            path_val,
            Some("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"),
            "PATH should be the fixed sandbox path, not inherited"
        );

        // Vars not in the safe list or config should not be present
        let has_random_env = envs
            .iter()
            .any(|(k, _)| k == "CARGO_HOME" || k == "RUSTUP_HOME");
        assert!(
            !has_random_env,
            "env_clear should have removed non-allowlisted vars"
        );
    }

    #[test]
    fn test_sandboxed_command_blocks_dangerous_env() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs)
            .with_workspace_root(std::env::temp_dir().join("astrid-test-workspace"));

        let config = ServerConfig::stdio("test", "echo")
            .with_env("LD_PRELOAD", "/evil.so")
            .with_env("SAFE_VAR", "ok");

        let cmd = manager
            .build_sandboxed_command("test", "echo", &config)
            .expect("should build command");

        let envs: Vec<_> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|v| {
                    (
                        k.to_string_lossy().to_string(),
                        v.to_string_lossy().to_string(),
                    )
                })
            })
            .collect();

        let has_ld_preload = envs.iter().any(|(k, _)| k == "LD_PRELOAD");
        assert!(!has_ld_preload, "LD_PRELOAD should be blocked");

        let has_safe = envs.iter().any(|(k, _)| k == "SAFE_VAR");
        assert!(has_safe, "safe config env should pass through");
    }

    #[test]
    fn test_writable_root_priority_cwd_first() {
        let cwd_dir = std::env::temp_dir().join("astrid-test-cwd");
        let ws_dir = std::env::temp_dir().join("astrid-test-ws");

        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs).with_workspace_root(ws_dir.clone());

        let mut config = ServerConfig::stdio("test", "echo");
        config.cwd = Some(cwd_dir.clone());

        let cmd = manager
            .build_sandboxed_command("test", "echo", &config)
            .expect("should build command");

        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        let args_joined = args.join(" ");

        let cwd_str = cwd_dir.to_string_lossy().to_string();
        assert!(
            args_joined.contains(&cwd_str),
            "writable root should be {cwd_str} (config.cwd wins), got args: {args_joined}"
        );
    }

    #[test]
    fn test_writable_root_priority_workspace_second() {
        let ws_dir = std::env::temp_dir().join("astrid-test-ws2");

        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs).with_workspace_root(ws_dir.clone());

        let config = ServerConfig::stdio("test", "echo");
        // No cwd set, should fall back to workspace_root

        let cmd = manager
            .build_sandboxed_command("test", "echo", &config)
            .expect("should build command");

        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        let args_joined = args.join(" ");

        let ws_str = ws_dir.to_string_lossy().to_string();
        assert!(
            args_joined.contains(&ws_str),
            "writable root should be {ws_str} (workspace_root fallback), got args: {args_joined}"
        );
    }

    #[test]
    fn test_resolve_astrid_home_succeeds() {
        // Should succeed as long as $HOME is set (which it is in test environments)
        let result = ServerManager::resolve_astrid_home();
        assert!(result.is_ok(), "should resolve astrid home from $HOME");

        let path = result.expect("already checked");
        assert!(
            path.to_string_lossy().ends_with(".astrid"),
            "path should end with .astrid, got: {}",
            path.display()
        );
    }

    #[test]
    fn test_relative_allowed_paths_rejected() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs)
            .with_workspace_root(std::env::temp_dir().join("astrid-test-workspace"));

        let config = ServerConfig::stdio("test", "echo")
            .with_read_path(std::path::PathBuf::from("relative/path"));

        let result = manager.build_sandboxed_command("test", "echo", &config);
        assert!(
            matches!(result, Err(McpError::ConfigError(_))),
            "relative allowed_read_paths should be rejected"
        );

        let config = ServerConfig::stdio("test", "echo")
            .with_write_path(std::path::PathBuf::from("another/relative"));

        let result = manager.build_sandboxed_command("test", "echo", &config);
        assert!(
            matches!(result, Err(McpError::ConfigError(_))),
            "relative allowed_write_paths should be rejected"
        );
    }

    #[test]
    fn test_double_quote_in_paths_rejected() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs)
            .with_workspace_root(std::env::temp_dir().join("astrid-test-workspace"));

        let config = ServerConfig::stdio("test", "echo")
            .with_read_path(std::path::PathBuf::from("/data/tricky\"path"));

        let result = manager.build_sandboxed_command("test", "echo", &config);
        assert!(
            matches!(result, Err(McpError::ConfigError(_))),
            "paths with double-quotes should be rejected to prevent SBPL injection"
        );

        let config = ServerConfig::stdio("test", "echo")
            .with_write_path(std::path::PathBuf::from("/output/also\"bad"));

        let result = manager.build_sandboxed_command("test", "echo", &config);
        assert!(
            matches!(result, Err(McpError::ConfigError(_))),
            "write paths with double-quotes should also be rejected"
        );
    }

    #[test]
    fn test_with_workspace_root() {
        let configs = ServersConfig::default();
        let manager = ServerManager::new(configs)
            .with_workspace_root(std::path::PathBuf::from("/my/workspace"));

        assert_eq!(
            manager.workspace_root,
            Some(std::path::PathBuf::from("/my/workspace"))
        );
    }
}
