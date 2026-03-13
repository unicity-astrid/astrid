//! MCP server configuration.
//!
//! Configuration is loaded from `~/.astrid/servers.toml`.

use astrid_crypto::ContentHash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{McpError, McpResult};

fn default_true() -> bool {
    true
}

/// Transport type for MCP servers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Standard I/O (spawn child process).
    #[default]
    Stdio,
    /// Server-Sent Events (HTTP streaming).
    Sse,
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio => f.write_str("stdio"),
            Self::Sse => f.write_str("sse"),
        }
    }
}

/// Policy for restarting a server when it dies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Never restart (default).
    #[default]
    Never,
    /// Restart on failure, up to `max_retries` times.
    OnFailure {
        /// Maximum number of restart attempts.
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
    /// Always restart (no retry limit).
    Always,
}

fn default_max_retries() -> u32 {
    3
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server name (unique identifier).
    #[serde(skip)]
    pub name: String,
    /// Transport type.
    #[serde(default)]
    pub transport: Transport,
    /// Command to run (for stdio transport).
    pub command: Option<String>,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// URL for SSE transport.
    pub url: Option<String>,
    /// Expected binary hash (sha256:...) for verification.
    pub binary_hash: Option<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
    /// Whether to auto-start on session begin.
    #[serde(default)]
    pub auto_start: bool,
    /// Description for users.
    pub description: Option<String>,
    /// Whether this server is trusted (runs natively vs WASM sandbox).
    ///
    /// When `false` (default), the server process is wrapped in an OS-level
    /// sandbox (bwrap on Linux, sandbox-exec on macOS) that restricts
    /// filesystem and optionally network access.
    #[serde(default)]
    pub trusted: bool,
    /// Whether to allow network access when sandboxed.
    ///
    /// Only relevant when `trusted` is `false`. Default: `true` because
    /// MCP servers commonly need network access (npm packages, API calls).
    #[serde(default = "default_true")]
    pub allow_network: bool,
    /// Additional paths the server can read (beyond OS defaults).
    ///
    /// Only relevant when `trusted` is `false`.
    #[serde(default)]
    pub allowed_read_paths: Vec<PathBuf>,
    /// Additional paths the server can write to (beyond its workspace root).
    ///
    /// Only relevant when `trusted` is `false`.
    #[serde(default)]
    pub allowed_write_paths: Vec<PathBuf>,
    /// Restart policy when the server process dies.
    #[serde(default)]
    pub restart_policy: RestartPolicy,
}

impl ServerConfig {
    /// Create a stdio server config.
    #[must_use]
    pub fn stdio(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: Transport::Stdio,
            command: Some(command.into()),
            args: Vec::new(),
            url: None,
            binary_hash: None,
            env: HashMap::new(),
            cwd: None,
            auto_start: false,
            description: None,
            trusted: false,
            allow_network: true,
            allowed_read_paths: Vec::new(),
            allowed_write_paths: Vec::new(),
            restart_policy: RestartPolicy::Never,
        }
    }

    /// Create an SSE server config.
    #[must_use]
    pub fn sse(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: Transport::Sse,
            command: None,
            args: Vec::new(),
            url: Some(url.into()),
            binary_hash: None,
            env: HashMap::new(),
            cwd: None,
            auto_start: false,
            description: None,
            trusted: false,
            allow_network: true,
            allowed_read_paths: Vec::new(),
            allowed_write_paths: Vec::new(),
            restart_policy: RestartPolicy::Never,
        }
    }

    /// Mark this server as trusted (native execution).
    #[must_use]
    pub fn trusted(mut self) -> Self {
        self.trusted = true;
        self
    }

    /// Add arguments.
    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Set binary hash.
    #[must_use]
    pub fn with_hash(mut self, hash: impl Into<String>) -> Self {
        self.binary_hash = Some(hash.into());
        self
    }

    /// Add environment variable.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set auto-start.
    #[must_use]
    pub fn auto_start(mut self) -> Self {
        self.auto_start = true;
        self
    }

    /// Set description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set restart policy.
    #[must_use]
    pub fn with_restart_policy(mut self, policy: RestartPolicy) -> Self {
        self.restart_policy = policy;
        self
    }

    /// Set whether network access is allowed when sandboxed.
    #[must_use]
    pub fn with_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    /// Add an additional readable path for sandboxed execution.
    #[must_use]
    pub fn with_read_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.allowed_read_paths.push(path.into());
        self
    }

    /// Add an additional writable path for sandboxed execution.
    #[must_use]
    pub fn with_write_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.allowed_write_paths.push(path.into());
        self
    }

    /// Verify binary hash if configured.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The binary cannot be found
    /// - The binary cannot be read
    /// - The hash does not match the expected value
    pub fn verify_binary(&self) -> McpResult<()> {
        let Some(expected) = &self.binary_hash else {
            return Ok(()); // No hash configured, skip verification
        };

        let Some(command) = &self.command else {
            return Ok(()); // No command to verify
        };

        // Find the binary path
        let binary_path = which::which(command)
            .map_err(|e| McpError::ConfigError(format!("Cannot find binary {command}: {e}")))?;

        // Read and hash the binary
        let binary_data = std::fs::read(&binary_path)?;
        let actual_hash = ContentHash::hash(&binary_data);
        let actual_str = format!("sha256:{}", actual_hash.to_hex());

        if expected != &actual_str {
            return Err(McpError::BinaryHashMismatch {
                name: self.name.clone(),
                expected: expected.clone(),
                actual: actual_str,
            });
        }

        Ok(())
    }
}

/// Configuration file for all MCP servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServersConfig {
    /// Server configurations.
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
    /// Timeout for graceful shutdown of MCP server sessions.
    ///
    /// Comes from `gateway.shutdown_timeout_secs` via the config bridge;
    /// skipped during (de)serialization because it is not part of
    /// `servers.toml`.
    #[serde(skip)]
    pub shutdown_timeout: std::time::Duration,
}

impl ServersConfig {
    /// Load configuration from a file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: impl AsRef<Path>) -> McpResult<Self> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let mut config: Self = toml::from_str(&content)
            .map_err(|e| McpError::ConfigError(format!("Invalid config: {e}")))?;

        // Set names from keys
        for (name, server) in &mut config.servers {
            server.name.clone_from(name);
        }

        Ok(config)
    }

    /// Load from the default location (`~/.astrid/servers.toml`).
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration directory cannot be determined
    /// or the file cannot be read.
    pub fn load_default() -> McpResult<Self> {
        let config_path = Self::default_path()?;

        if config_path.exists() {
            Self::load(&config_path)
        } else {
            Ok(Self::default())
        }
    }

    /// Get the default config path.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration directory cannot be determined.
    pub fn default_path() -> McpResult<PathBuf> {
        let home = astrid_core::dirs::AstridHome::resolve().map_err(|e| {
            McpError::ConfigError(format!("Cannot determine config directory: {e}"))
        })?;

        Ok(home.servers_config_path())
    }

    /// Save configuration to a file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written or serialized.
    pub fn save(&self, path: impl AsRef<Path>) -> McpResult<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| McpError::SerializationError(e.to_string()))?;

        // Ensure parent directory exists
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get a server config by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ServerConfig> {
        self.servers.get(name)
    }

    /// Add a server config.
    pub fn add(&mut self, config: ServerConfig) {
        self.servers.insert(config.name.clone(), config);
    }

    /// Remove a server config.
    pub fn remove(&mut self, name: &str) -> Option<ServerConfig> {
        self.servers.remove(name)
    }

    /// List all server names.
    #[must_use]
    pub fn list(&self) -> Vec<&str> {
        self.servers.keys().map(String::as_str).collect()
    }

    /// Get servers configured for auto-start.
    #[must_use]
    pub fn auto_start_servers(&self) -> Vec<&ServerConfig> {
        self.servers.values().filter(|s| s.auto_start).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_stdio() {
        let config = ServerConfig::stdio("filesystem", "npx")
            .with_args(["-y", "@anthropics/mcp-server-filesystem", "/tmp"])
            .with_env("DEBUG", "true")
            .auto_start();

        assert_eq!(config.name, "filesystem");
        assert_eq!(config.transport, Transport::Stdio);
        assert!(config.auto_start);
    }

    #[test]
    fn test_server_config_sse() {
        let config = ServerConfig::sse("remote", "https://example.com/mcp");

        assert_eq!(config.transport, Transport::Sse);
        assert_eq!(config.url, Some("https://example.com/mcp".to_string()));
    }

    #[test]
    fn test_config_parse() {
        let toml = r#"
[servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@anthropics/mcp-server-filesystem", "/home/user"]
auto_start = true

[servers.memory]
transport = "stdio"
command = "npx"
args = ["-y", "@anthropics/mcp-server-memory"]
"#;

        let config: ServersConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.servers.len(), 2);
        assert!(config.servers.contains_key("filesystem"));
        assert!(config.servers.contains_key("memory"));
    }

    #[test]
    fn test_auto_start_servers() {
        let mut config = ServersConfig::default();
        config.add(ServerConfig::stdio("server1", "cmd1").auto_start());
        config.add(ServerConfig::stdio("server2", "cmd2"));

        let auto_start = config.auto_start_servers();
        assert_eq!(auto_start.len(), 1);
        assert_eq!(auto_start[0].name, "server1");
    }

    #[test]
    fn test_sandbox_config_defaults() {
        let config = ServerConfig::stdio("test", "cmd");
        assert!(config.allow_network, "allow_network should default to true");
        assert!(config.allowed_read_paths.is_empty());
        assert!(config.allowed_write_paths.is_empty());
        assert!(!config.trusted);
    }

    #[test]
    fn test_sandbox_fields_parse_from_toml() {
        let toml = r#"
[servers.sandboxed]
command = "npx"
args = ["-y", "@anthropics/mcp-server-filesystem", "/tmp"]
allow_network = false
allowed_read_paths = ["/data/shared"]
allowed_write_paths = ["/data/output"]

[servers.minimal]
command = "echo"
"#;

        let config: ServersConfig = toml::from_str(toml).unwrap();
        let sandboxed = &config.servers["sandboxed"];
        assert!(!sandboxed.allow_network);
        assert_eq!(
            sandboxed.allowed_read_paths,
            vec![PathBuf::from("/data/shared")]
        );
        assert_eq!(
            sandboxed.allowed_write_paths,
            vec![PathBuf::from("/data/output")]
        );

        // Minimal config should have defaults
        let minimal = &config.servers["minimal"];
        assert!(minimal.allow_network);
        assert!(minimal.allowed_read_paths.is_empty());
        assert!(minimal.allowed_write_paths.is_empty());
    }

    #[test]
    fn test_sandbox_builder_methods() {
        let config = ServerConfig::stdio("test", "cmd")
            .with_network(false)
            .with_read_path("/data")
            .with_write_path("/output");

        assert!(!config.allow_network);
        assert_eq!(config.allowed_read_paths, vec![PathBuf::from("/data")]);
        assert_eq!(config.allowed_write_paths, vec![PathBuf::from("/output")]);
    }

    #[test]
    fn test_restart_policy_parse() {
        let toml = r#"
[servers.always]
command = "cmd1"
restart_policy = "always"

[servers.never]
command = "cmd2"
restart_policy = "never"

[servers.on_failure]
command = "cmd3"

[servers.on_failure.restart_policy]
on_failure = { max_retries = 5 }

[servers.default]
command = "cmd4"
"#;

        let config: ServersConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.servers["always"].restart_policy,
            RestartPolicy::Always
        );
        assert_eq!(config.servers["never"].restart_policy, RestartPolicy::Never);
        assert_eq!(
            config.servers["on_failure"].restart_policy,
            RestartPolicy::OnFailure { max_retries: 5 }
        );
        assert_eq!(
            config.servers["default"].restart_policy,
            RestartPolicy::Never
        );
    }
}
