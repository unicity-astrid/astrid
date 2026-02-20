//! Gateway configuration.

use crate::error::{GatewayError, GatewayResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Main gateway configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Gateway settings.
    #[serde(default)]
    pub gateway: GatewaySettings,

    /// Default agent configuration.
    #[serde(default)]
    pub defaults: AgentDefaults,

    /// Named agent configurations.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,

    /// Timeout configuration.
    #[serde(default)]
    pub timeouts: TimeoutConfig,

    /// Retry configuration for transient failures.
    #[serde(default)]
    pub retry: RetrySettings,

    /// Session configuration.
    #[serde(default)]
    pub sessions: SessionConfig,

    /// Pre-declared connector plugins to validate at startup.
    #[serde(default)]
    pub connectors: Vec<ConnectorConfig>,

    /// Pre-configured platform identity links applied at every startup.
    #[serde(default)]
    pub identity_links: Vec<IdentityLinkConfig>,
}

/// Pre-declared connector plugin entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectorConfig {
    /// Plugin ID (e.g. "openclaw-telegram").
    pub plugin: String,
    /// Expected connector profile: "chat", "interactive", "notify", or "bridge".
    pub profile: String,
}

/// A single pre-configured identity link.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdentityLinkConfig {
    /// Platform identifier (e.g. "telegram", "discord").
    pub platform: String,
    /// Platform-specific user ID.
    pub platform_user_id: String,
    /// Astrid user to link — UUID string or display name.
    pub astrid_user: String,
    /// Link verification method. Only "admin" is currently supported.
    pub method: String,
}

/// Retry settings for the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySettings {
    /// Maximum retry attempts for LLM requests.
    #[serde(default = "default_llm_retries")]
    pub llm_max_attempts: u32,

    /// Maximum retry attempts for MCP connections.
    #[serde(default = "default_mcp_retries")]
    pub mcp_max_attempts: u32,

    /// Initial retry delay in milliseconds.
    #[serde(default = "default_initial_delay_ms")]
    pub initial_delay_ms: u64,

    /// Maximum retry delay in milliseconds.
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u64,
}

impl Default for RetrySettings {
    fn default() -> Self {
        Self {
            llm_max_attempts: default_llm_retries(),
            mcp_max_attempts: default_mcp_retries(),
            initial_delay_ms: default_initial_delay_ms(),
            max_delay_ms: default_max_delay_ms(),
        }
    }
}

impl RetrySettings {
    /// Build a `RetryConfig` for LLM operations.
    #[must_use]
    pub fn llm_retry_config(&self) -> astrid_core::RetryConfig {
        astrid_core::RetryConfig::new(
            self.llm_max_attempts,
            std::time::Duration::from_millis(self.initial_delay_ms),
            std::time::Duration::from_millis(self.max_delay_ms),
            2.0,
        )
        .with_jitter(0.1)
    }

    /// Build a `RetryConfig` for MCP operations.
    #[must_use]
    pub fn mcp_retry_config(&self) -> astrid_core::RetryConfig {
        astrid_core::RetryConfig::new(
            self.mcp_max_attempts,
            std::time::Duration::from_millis(self.initial_delay_ms),
            std::time::Duration::from_millis(self.max_delay_ms),
            2.0,
        )
        .with_jitter(0.2)
    }
}

/// Gateway-level settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySettings {
    /// Path to secrets file.
    pub secrets_file: Option<String>,

    /// State directory for persistence.
    #[serde(default = "default_state_dir")]
    pub state_dir: String,

    /// Enable hot-reload of configuration.
    #[serde(default = "default_true")]
    pub hot_reload: bool,

    /// Health check interval in seconds.
    #[serde(default = "default_health_interval")]
    pub health_interval_secs: u64,

    /// Graceful shutdown timeout in seconds.
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            secrets_file: None,
            state_dir: default_state_dir(),
            hot_reload: true,
            health_interval_secs: default_health_interval(),
            shutdown_timeout_secs: default_shutdown_timeout(),
        }
    }
}

/// Agent identity configuration (gateway-local mirror of `astrid_tools::SparkConfig`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SparkConfig {
    /// Agent's name.
    pub callsign: String,
    /// Role archetype.
    pub class: String,
    /// Personality energy.
    pub aura: String,
    /// Communication style.
    pub signal: String,
    /// Soul/philosophy.
    pub core: String,
}

impl SparkConfig {
    /// Returns `true` when all fields are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.callsign.is_empty()
            && self.class.is_empty()
            && self.aura.is_empty()
            && self.signal.is_empty()
            && self.core.is_empty()
    }
}

/// Default agent settings applied to all agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefaults {
    /// Default model configuration.
    #[serde(default)]
    pub model: ModelConfig,

    /// Default subagent pool settings.
    #[serde(default)]
    pub subagents: SubAgentDefaults,

    /// Default system prompt.
    pub system_prompt: Option<String>,

    /// Maximum context tokens.
    #[serde(default = "default_max_context")]
    pub max_context_tokens: usize,

    /// Default agent identity (spark).
    #[serde(default)]
    pub spark: Option<SparkConfig>,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            model: ModelConfig::default(),
            subagents: SubAgentDefaults::default(),
            system_prompt: None,
            max_context_tokens: default_max_context(),
            spark: None,
        }
    }
}

/// Individual agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name/identifier.
    pub name: String,

    /// Optional description.
    pub description: Option<String>,

    /// Model configuration (overrides defaults).
    pub model: Option<ModelConfig>,

    /// System prompt (overrides defaults).
    pub system_prompt: Option<String>,

    /// Maximum context tokens (overrides defaults).
    pub max_context_tokens: Option<usize>,

    /// Subagent configuration (overrides defaults).
    pub subagents: Option<SubAgentDefaults>,

    /// Timeout overrides for this agent.
    pub timeouts: Option<TimeoutConfig>,

    /// Channel bindings for this agent.
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,

    /// Auto-start this agent on gateway startup.
    #[serde(default)]
    pub auto_start: bool,

    /// Agent identity (spark) override for this agent.
    pub spark: Option<SparkConfig>,
}

/// Channel configuration for routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Channel type (cli, discord, web, etc.).
    pub channel_type: String,

    /// Scope (dm, guild, channel, etc.).
    pub scope: Option<String>,

    /// Identifier pattern (user id, guild id, etc.).
    pub identifier: Option<String>,
}

/// Model configuration.
#[derive(Clone, Deserialize)]
pub struct ModelConfig {
    /// Provider name (claude, openai, `lm_studio`, etc.).
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Model name.
    #[serde(default = "default_model")]
    pub model: String,

    /// API key (can use ${secrets.key} or ${VAR}).
    #[serde(skip_serializing)]
    pub api_key: Option<String>,

    /// Base URL for API.
    #[serde(skip_serializing)]
    pub base_url: Option<String>,

    /// Maximum tokens for responses.
    pub max_tokens: Option<usize>,

    /// Temperature.
    pub temperature: Option<f64>,
}

impl std::fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelConfig")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("has_api_key", &self.api_key.is_some())
            .field("has_base_url", &self.base_url.is_some())
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .finish()
    }
}

impl Serialize for ModelConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ModelConfig", 4)?;
        state.serialize_field("provider", &self.provider)?;
        state.serialize_field("model", &self.model)?;
        // api_key and base_url are intentionally omitted.
        state.serialize_field("max_tokens", &self.max_tokens)?;
        state.serialize_field("temperature", &self.temperature)?;
        state.end()
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            api_key: None,
            base_url: None,
            max_tokens: None,
            temperature: None,
        }
    }
}

/// Subagent pool defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentDefaults {
    /// Maximum concurrent subagents.
    #[serde(default = "default_max_subagents")]
    pub max_concurrent: usize,

    /// Subagent timeout in seconds.
    #[serde(default = "default_subagent_timeout")]
    pub timeout_secs: u64,

    /// Maximum subagent depth (for recursive calls).
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
}

impl Default for SubAgentDefaults {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_subagents(),
            timeout_secs: default_subagent_timeout(),
            max_depth: default_max_depth(),
        }
    }
}

/// Timeout configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// LLM request timeout in seconds.
    #[serde(default = "default_request_timeout")]
    pub request_secs: u64,

    /// Tool call timeout in seconds.
    #[serde(default = "default_tool_timeout")]
    pub tool_secs: u64,

    /// Subagent timeout in seconds.
    #[serde(default = "default_subagent_timeout")]
    pub subagent_secs: u64,

    /// MCP server connection timeout in seconds.
    #[serde(default = "default_mcp_connect_timeout")]
    pub mcp_connect_secs: u64,

    /// Approval wait timeout in seconds.
    #[serde(default = "default_approval_timeout")]
    pub approval_secs: u64,

    /// Idle session timeout in seconds.
    #[serde(default = "default_idle_timeout")]
    pub idle_session_secs: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            request_secs: default_request_timeout(),
            tool_secs: default_tool_timeout(),
            subagent_secs: default_subagent_timeout(),
            mcp_connect_secs: default_mcp_connect_timeout(),
            approval_secs: default_approval_timeout(),
            idle_session_secs: default_idle_timeout(),
        }
    }
}

/// Session configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Maximum sessions per user.
    #[serde(default = "default_max_sessions")]
    pub max_per_user: usize,

    /// Session history limit (messages).
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,

    /// Auto-save interval in seconds.
    #[serde(default = "default_save_interval")]
    pub save_interval_secs: u64,

    /// Enable session persistence.
    #[serde(default = "default_true")]
    pub persist: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_per_user: default_max_sessions(),
            history_limit: default_history_limit(),
            save_interval_secs: default_save_interval(),
            persist: true,
        }
    }
}

impl GatewayConfig {
    /// Load configuration from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load<P: AsRef<Path>>(path: P) -> GatewayResult<Self> {
        let contents = std::fs::read_to_string(path.as_ref())?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Load configuration from the default location (`~/.astrid/gateway.toml`).
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be determined or the file cannot be parsed.
    pub fn load_default() -> GatewayResult<Self> {
        let home = astrid_core::dirs::AstridHome::resolve().map_err(|e| {
            GatewayError::Config(format!("could not determine config directory: {e}"))
        })?;

        let config_path = home.gateway_config_path();

        if config_path.exists() {
            Self::load(&config_path)
        } else {
            Ok(Self::default())
        }
    }

    /// Get agent config by name, falling back to defaults.
    #[must_use]
    pub fn agent_config(&self, name: &str) -> Option<&AgentConfig> {
        self.agents.get(name)
    }

    /// Get effective model config for an agent.
    #[must_use]
    pub fn effective_model(&self, agent_name: &str) -> ModelConfig {
        self.agents
            .get(agent_name)
            .and_then(|agent| agent.model.clone())
            .unwrap_or_else(|| self.defaults.model.clone())
    }

    /// Get effective system prompt for an agent.
    #[must_use]
    pub fn effective_system_prompt(&self, agent_name: &str) -> Option<String> {
        self.agents
            .get(agent_name)
            .and_then(|agent| agent.system_prompt.clone())
            .or_else(|| self.defaults.system_prompt.clone())
    }

    /// Get effective max context tokens for an agent.
    #[must_use]
    pub fn effective_max_context(&self, agent_name: &str) -> usize {
        self.agents
            .get(agent_name)
            .and_then(|agent| agent.max_context_tokens)
            .unwrap_or(self.defaults.max_context_tokens)
    }

    /// Get effective timeouts for an agent (agent overrides fall back to global).
    #[must_use]
    pub fn effective_timeouts(&self, agent_name: &str) -> TimeoutConfig {
        self.agents
            .get(agent_name)
            .and_then(|agent| agent.timeouts.clone())
            .unwrap_or_else(|| self.timeouts.clone())
    }

    /// Get effective spark identity for an agent (agent overrides fall back to defaults).
    #[must_use]
    pub fn effective_spark(&self, agent_name: &str) -> Option<SparkConfig> {
        self.agents
            .get(agent_name)
            .and_then(|agent| agent.spark.clone())
            .or_else(|| self.defaults.spark.clone())
            .filter(|s| !s.is_empty())
    }

    /// Get auto-start agents.
    #[must_use]
    pub fn auto_start_agents(&self) -> Vec<&str> {
        self.agents
            .iter()
            .filter(|(_, config)| config.auto_start)
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

// Default value functions
fn default_state_dir() -> String {
    astrid_core::dirs::AstridHome::resolve().map_or_else(
        |_| "~/.astrid/state".into(),
        |home| home.state_dir().to_string_lossy().to_string(),
    )
}

fn default_true() -> bool {
    true
}

fn default_health_interval() -> u64 {
    30
}

fn default_shutdown_timeout() -> u64 {
    30
}

fn default_max_context() -> usize {
    100_000
}

fn default_provider() -> String {
    "claude".into()
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".into()
}

fn default_max_subagents() -> usize {
    5
}

fn default_subagent_timeout() -> u64 {
    300
}

fn default_max_depth() -> usize {
    3
}

fn default_llm_retries() -> u32 {
    3
}

fn default_mcp_retries() -> u32 {
    5
}

fn default_initial_delay_ms() -> u64 {
    100
}

fn default_max_delay_ms() -> u64 {
    10_000
}

fn default_mcp_connect_timeout() -> u64 {
    10
}

fn default_request_timeout() -> u64 {
    120
}

fn default_tool_timeout() -> u64 {
    60
}

fn default_approval_timeout() -> u64 {
    300
}

fn default_idle_timeout() -> u64 {
    3600
}

fn default_max_sessions() -> usize {
    10
}

fn default_history_limit() -> usize {
    100
}

fn default_save_interval() -> u64 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GatewayConfig::default();
        assert!(config.agents.is_empty());
        assert_eq!(config.defaults.max_context_tokens, 100_000);
        assert_eq!(config.timeouts.request_secs, 120);
        assert_eq!(config.timeouts.tool_secs, 60);
        assert_eq!(config.timeouts.subagent_secs, 300);
        assert_eq!(config.timeouts.mcp_connect_secs, 10);
        assert_eq!(config.timeouts.approval_secs, 300);
    }

    #[test]
    fn test_parse_config() {
        let toml = r#"
            [gateway]
            state_dir = "/tmp/astrid"
            hot_reload = false

            [defaults.model]
            provider = "openai"
            model = "gpt-4"

            [agents.assistant]
            name = "assistant"
            description = "Main assistant"
            auto_start = true

            [[agents.assistant.channels]]
            channel_type = "cli"
        "#;

        let config: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.gateway.state_dir, "/tmp/astrid");
        assert!(!config.gateway.hot_reload);
        assert_eq!(config.defaults.model.provider, "openai");
        assert!(config.agents.contains_key("assistant"));
        assert!(config.agents["assistant"].auto_start);
    }

    #[test]
    fn test_effective_model() {
        let mut config = GatewayConfig::default();
        config.agents.insert(
            "custom".into(),
            AgentConfig {
                name: "custom".into(),
                description: None,
                model: Some(ModelConfig {
                    provider: "openai".into(),
                    model: "gpt-4".into(),
                    ..Default::default()
                }),
                system_prompt: None,
                max_context_tokens: None,
                subagents: None,
                timeouts: None,
                channels: vec![],
                auto_start: false,
                spark: None,
            },
        );

        // Agent with custom model should use it
        let model = config.effective_model("custom");
        assert_eq!(model.provider, "openai");

        // Unknown agent should use defaults
        let model = config.effective_model("unknown");
        assert_eq!(model.provider, "claude");
    }

    #[test]
    fn test_effective_timeouts() {
        let mut config = GatewayConfig::default();
        config.agents.insert(
            "fast".into(),
            AgentConfig {
                name: "fast".into(),
                description: None,
                model: None,
                system_prompt: None,
                max_context_tokens: None,
                subagents: None,
                timeouts: Some(TimeoutConfig {
                    request_secs: 30,
                    ..Default::default()
                }),
                channels: vec![],
                auto_start: false,
                spark: None,
            },
        );

        // Agent with custom timeouts
        let t = config.effective_timeouts("fast");
        assert_eq!(t.request_secs, 30);

        // Unknown agent falls back to global
        let t = config.effective_timeouts("unknown");
        assert_eq!(t.request_secs, 120);
    }

    #[test]
    fn test_auto_start_agents() {
        let mut config = GatewayConfig::default();
        config.agents.insert(
            "agent1".into(),
            AgentConfig {
                name: "agent1".into(),
                description: None,
                model: None,
                system_prompt: None,
                max_context_tokens: None,
                subagents: None,
                timeouts: None,
                channels: vec![],
                auto_start: true,
                spark: None,
            },
        );
        config.agents.insert(
            "agent2".into(),
            AgentConfig {
                name: "agent2".into(),
                description: None,
                model: None,
                system_prompt: None,
                max_context_tokens: None,
                subagents: None,
                timeouts: None,
                channels: vec![],
                auto_start: false,
                spark: None,
            },
        );

        let auto_start = config.auto_start_agents();
        assert_eq!(auto_start.len(), 1);
        assert!(auto_start.contains(&"agent1"));
    }
}
