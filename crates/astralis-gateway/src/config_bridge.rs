//! Bridge from unified `astralis_config::Config` to `GatewayConfig`.
//!
//! Converts the unified config into the gateway-specific `GatewayConfig` type
//! so the gateway runtime can be constructed from the unified config chain
//! instead of loading a separate `gateway.toml`.

use std::collections::HashMap;

use crate::config::{
    AgentDefaults, GatewayConfig, GatewaySettings, ModelConfig, RetrySettings, SessionConfig,
    SubAgentDefaults, TimeoutConfig,
};

/// Convert a unified [`astralis_config::Config`] into a [`GatewayConfig`].
///
/// The `agents` map is left empty — it is gateway-specific (multi-agent
/// routing) and should be populated from gateway-specific config if needed.
#[must_use]
pub fn from_unified_config(cfg: &astralis_config::Config) -> GatewayConfig {
    let gateway = GatewaySettings {
        secrets_file: cfg.gateway.secrets_file.clone(),
        state_dir: cfg
            .gateway
            .state_dir
            .clone()
            .unwrap_or_else(default_state_dir),
        hot_reload: cfg.gateway.hot_reload,
        health_interval_secs: cfg.gateway.health_interval_secs,
        shutdown_timeout_secs: cfg.gateway.shutdown_timeout_secs,
    };

    let defaults = AgentDefaults {
        model: ModelConfig {
            provider: cfg.model.provider.clone(),
            model: cfg.model.model.clone(),
            api_key: cfg.model.api_key.clone(),
            base_url: cfg.model.api_url.clone(),
            max_tokens: Some(cfg.model.max_tokens),
            temperature: Some(cfg.model.temperature),
        },
        subagents: SubAgentDefaults {
            max_concurrent: cfg.subagents.max_concurrent,
            timeout_secs: cfg.subagents.timeout_secs,
            max_depth: cfg.subagents.max_depth,
        },
        system_prompt: Some(cfg.runtime.system_prompt.clone()),
        max_context_tokens: cfg.runtime.max_context_tokens,
    };

    let timeouts = TimeoutConfig {
        request_secs: cfg.timeouts.request_secs,
        tool_secs: cfg.timeouts.tool_secs,
        subagent_secs: cfg.timeouts.subagent_secs,
        mcp_connect_secs: cfg.timeouts.mcp_connect_secs,
        approval_secs: cfg.timeouts.approval_secs,
        idle_session_secs: cfg.timeouts.idle_secs,
    };

    let retry = RetrySettings {
        llm_max_attempts: cfg.retry.llm_max_attempts,
        mcp_max_attempts: cfg.retry.mcp_max_attempts,
        initial_delay_ms: cfg.retry.initial_delay_ms,
        max_delay_ms: cfg.retry.max_delay_ms,
    };

    let sessions = SessionConfig {
        max_per_user: cfg.sessions.max_per_user,
        history_limit: cfg.sessions.history_limit,
        save_interval_secs: cfg.sessions.save_interval_secs,
        persist: cfg.sessions.persist,
    };

    GatewayConfig {
        gateway,
        defaults,
        agents: HashMap::new(),
        timeouts,
        retry,
        sessions,
    }
}

fn default_state_dir() -> String {
    directories::ProjectDirs::from("", "", "astralis").map_or_else(
        || "~/.astralis/state".into(),
        |dirs| dirs.data_dir().join("state").to_string_lossy().to_string(),
    )
}
