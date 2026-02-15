//! Daemon client — connects CLI to the running daemon via `WebSocket`.
//!
//! The CLI is a thin client: it connects to the daemon, creates/resumes sessions,
//! subscribes to events, and renders output. All heavy lifting happens in the daemon.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use astrid_core::{ApprovalDecision, ElicitationResponse, SessionId};
use astrid_gateway::DaemonServer;
use astrid_gateway::rpc::{
    AllowanceInfo, AstridRpcClient, AuditEntryInfo, BudgetInfo, DaemonEvent, DaemonStatus,
    McpServerInfo, PluginInfo, SessionInfo, ToolInfo,
};
use astrid_gateway::server::DaemonPaths;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};

/// A client that connects to the Astrid daemon.
pub struct DaemonClient {
    client: WsClient,
}

impl DaemonClient {
    /// Connect to the daemon, auto-starting it if necessary.
    ///
    /// Reads the port from `~/.astrid/daemon.port`. If the daemon isn't running,
    /// starts it as a background process and waits for it to become available.
    ///
    /// # Errors
    ///
    /// Returns an error if the daemon cannot be started or connected to.
    pub async fn connect() -> anyhow::Result<Self> {
        let paths = DaemonPaths::default_dir()?;

        // Check if daemon is running; if not, start it.
        if !DaemonServer::is_running(&paths) {
            Self::start_daemon(&paths).await?;
        }

        let port = DaemonServer::read_port(&paths)
            .ok_or_else(|| anyhow::anyhow!("Daemon port file not found"))?;

        let url = format!("ws://127.0.0.1:{port}");

        let client = WsClientBuilder::default()
            .connection_timeout(Duration::from_secs(5))
            .build(&url)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to daemon at {url}: {e}"))?;

        Ok(Self { client })
    }

    /// Start the daemon as a background process.
    ///
    /// Stderr is redirected to `~/.astrid/logs/daemon.log` so startup errors
    /// can be surfaced to the user if the daemon fails to come up.
    async fn start_daemon(paths: &DaemonPaths) -> anyhow::Result<()> {
        // Find the current executable — the daemon runs as `astrid daemon run`.
        let exe = std::env::current_exe()
            .map_err(|e| anyhow::anyhow!("Failed to find current executable: {e}"))?;

        // Ensure logs directory exists before daemon spawn.
        let log_file_path = paths.log_file();
        if let Some(parent) = log_file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("Failed to create logs directory: {e}"))?;
        }

        // Open log file for daemon stderr (truncate on each start).
        let log_file = std::fs::File::create(&log_file_path)
            .map_err(|e| anyhow::anyhow!("Failed to create daemon log file: {e}"))?;

        // Launch daemon in background with stderr captured.
        // Auto-started daemons use ephemeral mode so they shut down
        // automatically when all CLI clients disconnect.
        std::process::Command::new(&exe)
            .args(["daemon", "run", "--ephemeral"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(log_file))
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to start daemon: {e}"))?;

        // Wait for daemon to become available (poll port file).
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if DaemonServer::is_running(paths) && DaemonServer::read_port(paths).is_some() {
                return Ok(());
            }
        }

        // Daemon did not start — read log to surface the error.
        let hint = std::fs::read_to_string(&log_file_path)
            .ok()
            .and_then(|log| extract_startup_error(&log))
            .unwrap_or_default();

        let mut msg = String::from("Daemon did not start within 5 seconds.");
        if !hint.is_empty() {
            let _ = write!(msg, "\n  Error: {hint}");
        }
        let _ = write!(msg, "\n  Full log: {}", log_file_path.display());

        Err(anyhow::anyhow!("{msg}"))
    }

    /// Create a new session.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn create_session(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> anyhow::Result<SessionInfo> {
        let info = self.client.create_session(workspace_path).await?;
        Ok(info)
    }

    /// Resume an existing session.
    ///
    /// # Errors
    ///
    /// Returns an error if the session is not found or the RPC call fails.
    pub async fn resume_session(&self, session_id: SessionId) -> anyhow::Result<SessionInfo> {
        let info = self.client.resume_session(session_id).await?;
        Ok(info)
    }

    /// Send user input. Events arrive via subscription.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn send_input(&self, session_id: &SessionId, input: &str) -> anyhow::Result<()> {
        self.client
            .send_input(session_id.clone(), input.to_string())
            .await?;
        Ok(())
    }

    /// Respond to an approval request.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn send_approval(
        &self,
        session_id: &SessionId,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> anyhow::Result<()> {
        self.client
            .approval_response(session_id.clone(), request_id.to_string(), decision)
            .await?;
        Ok(())
    }

    /// Respond to an elicitation request.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn send_elicitation(
        &self,
        session_id: &SessionId,
        request_id: &str,
        response: ElicitationResponse,
    ) -> anyhow::Result<()> {
        self.client
            .elicitation_response(session_id.clone(), request_id.to_string(), response)
            .await?;
        Ok(())
    }

    /// Subscribe to session events.
    ///
    /// # Errors
    ///
    /// Returns an error if the subscription cannot be established.
    pub async fn subscribe_events(
        &self,
        session_id: &SessionId,
    ) -> anyhow::Result<jsonrpsee::core::client::Subscription<DaemonEvent>> {
        let sub = self.client.subscribe_events(session_id.clone()).await?;
        Ok(sub)
    }

    /// List active sessions.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn list_sessions(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> anyhow::Result<Vec<SessionInfo>> {
        let sessions = self.client.list_sessions(workspace_path).await?;
        Ok(sessions)
    }

    /// End a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn end_session(&self, session_id: &SessionId) -> anyhow::Result<()> {
        self.client.end_session(session_id.clone()).await?;
        Ok(())
    }

    /// List MCP servers and their status.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn list_servers(&self) -> anyhow::Result<Vec<McpServerInfo>> {
        let servers = self.client.list_servers().await?;
        Ok(servers)
    }

    /// Start a named MCP server via the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn start_server(&self, name: &str) -> anyhow::Result<()> {
        self.client.start_server(name.to_string()).await?;
        Ok(())
    }

    /// Stop a named MCP server via the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn stop_server(&self, name: &str) -> anyhow::Result<()> {
        self.client.stop_server(name.to_string()).await?;
        Ok(())
    }

    /// List registered plugins and their status via the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn list_plugins(&self) -> anyhow::Result<Vec<PluginInfo>> {
        let plugins = self.client.list_plugins().await?;
        Ok(plugins)
    }

    /// Load (or reload) a plugin by ID via the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn load_plugin(&self, plugin_id: &str) -> anyhow::Result<PluginInfo> {
        let info = self.client.load_plugin(plugin_id.to_string()).await?;
        Ok(info)
    }

    /// Unload a plugin by ID via the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn unload_plugin(&self, plugin_id: &str) -> anyhow::Result<()> {
        self.client.unload_plugin(plugin_id.to_string()).await?;
        Ok(())
    }

    /// List tools from all running MCP servers via the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn list_tools(&self) -> anyhow::Result<Vec<ToolInfo>> {
        let tools = self.client.list_tools().await?;
        Ok(tools)
    }

    /// Get daemon status.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn status(&self) -> anyhow::Result<DaemonStatus> {
        let status = self.client.status().await?;
        Ok(status)
    }

    /// Shutdown the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.client.shutdown().await?;
        Ok(())
    }

    /// Get budget information for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn session_budget(&self, session_id: &SessionId) -> anyhow::Result<BudgetInfo> {
        let info = self.client.session_budget(session_id.clone()).await?;
        Ok(info)
    }

    /// Get active allowances for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn session_allowances(
        &self,
        session_id: &SessionId,
    ) -> anyhow::Result<Vec<AllowanceInfo>> {
        let infos = self.client.session_allowances(session_id.clone()).await?;
        Ok(infos)
    }

    /// Get recent audit entries for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn session_audit(
        &self,
        session_id: &SessionId,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<AuditEntryInfo>> {
        let entries = self.client.session_audit(session_id.clone(), limit).await?;
        Ok(entries)
    }

    /// Cancel the currently running turn for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn cancel_turn(&self, session_id: &SessionId) -> anyhow::Result<()> {
        self.client.cancel_turn(session_id.clone()).await?;
        Ok(())
    }

    /// Explicitly save a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn save_session(&self, session_id: &SessionId) -> anyhow::Result<()> {
        self.client.save_session(session_id.clone()).await?;
        Ok(())
    }
}

/// Scan a daemon log (from the bottom) for the most relevant error line.
///
/// Looks for known patterns like `Error:`, `panicked`, `FATAL`, etc. and
/// returns the first match found scanning from the end. Returns `None` if
/// no recognizable error is found.
fn extract_startup_error(log: &str) -> Option<String> {
    let patterns = ["Error:", "ERROR", "panicked", "FATAL", "fatal error"];

    for line in log.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        for pat in &patterns {
            if trimmed.contains(pat) {
                return Some(trimmed.to_string());
            }
        }
    }

    // Fall back to the last non-empty line if no pattern matched.
    log.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
}
