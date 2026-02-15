//! MCP server lifecycle management.
//!
//! Handles starting, stopping, and managing MCP server processes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use astralis_core::retry::RetryConfig;

use rmcp::ServiceExt;
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;

use serde::{Deserialize, Serialize};

use crate::capabilities::CapabilitiesHandler;
use crate::capabilities::{AstralisClientHandler, ServerNotice};
use crate::config::{RestartPolicy, ServerConfig, ServersConfig, Transport};
use crate::error::{McpError, McpResult};
use crate::types::{ServerInfo, ToolDefinition};

use tokio::sync::mpsc;

/// Type alias for a running MCP client service.
type McpService = RunningService<RoleClient, AstralisClientHandler>;

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

/// Status snapshot for a single MCP server (used for reporting).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerStatus {
    /// Server name.
    pub name: String,
    /// Whether the server process is alive.
    pub alive: bool,
    /// Whether the server has completed the MCP handshake and is ready.
    pub ready: bool,
    /// Number of tools provided by this server.
    pub tool_count: usize,
    /// How many times this server has been restarted.
    pub restart_count: u32,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Manages MCP server lifecycles.
pub struct ServerManager {
    /// Server configurations.
    configs: ServersConfig,
    /// Running servers.
    running: Arc<RwLock<HashMap<String, RunningServer>>>,
    /// Timeout for graceful `close_with_timeout` during shutdown.
    shutdown_timeout: std::time::Duration,
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
        }
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

        // Build the tokio::process::Command
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&config.args);

        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }

        // Create transport (spawns the child process)
        let transport = TokioChildProcess::new(cmd).map_err(|e| McpError::ServerStartFailed {
            name: name.to_string(),
            reason: e.to_string(),
        })?;

        // Create the client handler and perform the MCP handshake
        let mut client_handler = AstralisClientHandler::new(name, handler);
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

    /// Get server info.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not running.
    pub async fn get_server_info(&self, name: &str) -> McpResult<Option<ServerInfo>> {
        let running = self.running.read().await;
        let server = running
            .get(name)
            .ok_or_else(|| McpError::ServerNotRunning {
                name: name.to_string(),
            })?;

        Ok(server.info.clone())
    }

    /// Update server info after connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not running.
    pub async fn set_server_info(&self, name: &str, info: ServerInfo) -> McpResult<()> {
        let mut running = self.running.write().await;
        let server = running
            .get_mut(name)
            .ok_or_else(|| McpError::ServerNotRunning {
                name: name.to_string(),
            })?;

        server.info = Some(info);
        server.ready = true;

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

    /// Get status snapshots for all running servers.
    pub async fn server_statuses(&self) -> Vec<McpServerStatus> {
        let running = self.running.read().await;
        running
            .values()
            .map(|s| McpServerStatus {
                name: s.config.name.clone(),
                alive: s.is_alive(),
                ready: s.ready,
                tool_count: s.tools.len(),
                restart_count: s.restart_count,
                description: s.config.description.clone(),
            })
            .collect()
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
            server.last_restart_attempt = Some(Instant::now() - std::time::Duration::from_secs(60));
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
}
