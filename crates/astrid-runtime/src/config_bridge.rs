//! Bridge from `astrid_config::Config` to domain types.
//!
//! The unified config crate has no dependencies on other internal crates.
//! This module provides conversion functions that translate config types into
//! the domain types used by the runtime, LLM provider, MCP client, etc.
//!
//! Both the CLI and the gateway daemon use this module so that config-to-domain
//! conversion happens exactly once, in one place.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use astrid_approval::budget::BudgetConfig;
use astrid_approval::policy::SecurityPolicy;
use astrid_config::{Config, RestartPolicyConfig};
use astrid_hooks::config::HooksConfig;
use astrid_llm::ProviderConfig;
use astrid_mcp::{RestartPolicy, ServerConfig, ServersConfig, Transport};
use astrid_telemetry::{LogConfig, LogFormat};
use astrid_workspace::EscapePolicy;

use astrid_tools::SparkConfig;

use crate::{RuntimeConfig, WorkspaceConfig, WorkspaceMode};

/// Convert config to [`RuntimeConfig`].
#[must_use]
pub fn to_runtime_config(cfg: &Config, workspace_root: &Path) -> RuntimeConfig {
    let workspace = to_workspace_config(cfg, workspace_root);

    // Convert [spark] section to SparkConfig (None if all fields empty).
    let spark_seed = if cfg.spark.is_empty() {
        None
    } else {
        Some(SparkConfig {
            callsign: cfg.spark.callsign.clone(),
            class: cfg.spark.class.clone(),
            aura: cfg.spark.aura.clone(),
            signal: cfg.spark.signal.clone(),
            core: cfg.spark.core.clone(),
        })
    };

    // Resolve spark file path from AstridHome.
    let spark_file = astrid_core::dirs::AstridHome::resolve()
        .ok()
        .map(|h| h.spark_path());

    RuntimeConfig {
        max_context_tokens: cfg.runtime.max_context_tokens,
        system_prompt: cfg.runtime.system_prompt.clone(),
        auto_summarize: cfg.runtime.auto_summarize,
        keep_recent_count: cfg.runtime.keep_recent_count,
        workspace,
        max_concurrent_subagents: cfg.subagents.max_concurrent,
        max_subagent_depth: cfg.subagents.max_depth,
        default_subagent_timeout: std::time::Duration::from_secs(cfg.subagents.timeout_secs),
        spark_seed,
        spark_file,
    }
}

/// Convert config to [`ProviderConfig`].
///
/// The API key comes from `cfg.model.api_key` which already includes env var
/// fallbacks applied by [`Config::load`]. No additional `std::env::var` call
/// is needed.
///
/// If no API key is available yet, the provider is created with an empty key.
/// The actual LLM provider will return [`ApiKeyNotConfigured`] on the first
/// call, which surfaces a clear error through `DaemonEvent::Error`.
#[must_use]
pub fn to_provider_config(cfg: &Config) -> ProviderConfig {
    let api_key = cfg.model.api_key.clone().unwrap_or_default();

    let mut provider = ProviderConfig::new(api_key, &cfg.model.model)
        .max_tokens(cfg.model.max_tokens)
        .temperature(cfg.model.temperature);

    if let Some(url) = &cfg.model.api_url {
        provider = provider.base_url(url);
    }

    if let Some(ctx) = cfg.model.context_window {
        provider = provider.context_window(ctx);
    }

    provider
}

/// Convert config to [`SecurityPolicy`].
#[must_use]
pub fn to_security_policy(cfg: &Config) -> SecurityPolicy {
    let policy = &cfg.security.policy;

    SecurityPolicy {
        blocked_tools: policy.blocked_tools.iter().cloned().collect::<HashSet<_>>(),
        approval_required_tools: policy
            .approval_required_tools
            .iter()
            .cloned()
            .collect::<HashSet<_>>(),
        allowed_paths: policy.allowed_paths.clone(),
        denied_paths: policy.denied_paths.clone(),
        allowed_hosts: policy.allowed_hosts.clone(),
        denied_hosts: policy.denied_hosts.clone(),
        max_argument_size: policy.max_argument_size,
        require_approval_for_delete: policy.require_approval_for_delete,
        require_approval_for_network: policy.require_approval_for_network,
        blocked_plugins: HashSet::new(),
    }
}

/// Convert config to [`BudgetConfig`].
#[must_use]
pub fn to_budget_config(cfg: &Config) -> BudgetConfig {
    BudgetConfig::new(cfg.budget.session_max_usd, cfg.budget.per_action_max_usd)
        .with_warn_at_percent(cfg.budget.warn_at_percent)
}

/// Convert config to [`ServersConfig`].
pub fn to_servers_config(cfg: &Config) -> ServersConfig {
    let mut servers = HashMap::new();

    for (name, section) in &cfg.servers {
        let transport = match section.transport.as_str() {
            "sse" => Transport::Sse,
            _ => Transport::Stdio,
        };

        let server = ServerConfig {
            name: name.clone(),
            transport,
            command: section.command.clone(),
            args: section.args.clone(),
            url: section.url.clone(),
            binary_hash: section.binary_hash.clone(),
            env: section.env.clone(),
            cwd: section.cwd.as_ref().map(PathBuf::from),
            auto_start: section.auto_start,
            description: section.description.clone(),
            trusted: section.trusted,
            restart_policy: convert_restart_policy(&section.restart_policy),
        };

        servers.insert(name.clone(), server);
    }

    ServersConfig {
        servers,
        shutdown_timeout: std::time::Duration::from_secs(cfg.gateway.shutdown_timeout_secs),
    }
}

/// Convert a config-layer restart policy to the domain type.
fn convert_restart_policy(policy: &RestartPolicyConfig) -> RestartPolicy {
    match policy {
        RestartPolicyConfig::Never => RestartPolicy::Never,
        RestartPolicyConfig::OnFailure { max_retries } => RestartPolicy::OnFailure {
            max_retries: *max_retries,
        },
        RestartPolicyConfig::Always => RestartPolicy::Always,
    }
}

/// Convert config to [`HooksConfig`].
#[must_use]
pub fn to_hooks_config(cfg: &Config) -> HooksConfig {
    HooksConfig {
        enabled: cfg.hooks.enabled,
        default_timeout_secs: cfg.hooks.default_timeout_secs,
        max_hooks: cfg.hooks.max_hooks,
        hook_directories: Vec::new(),
        profile: None,
        allow_async_hooks: cfg.hooks.allow_async_hooks,
        allow_wasm_hooks: cfg.hooks.allow_wasm_hooks,
        allow_agent_hooks: cfg.hooks.allow_agent_hooks,
        allow_http_hooks: cfg.hooks.allow_http_hooks,
        allow_command_hooks: cfg.hooks.allow_command_hooks,
        global_env: HashMap::new(),
        default_working_dir: None,
    }
}

/// Convert config to [`LogConfig`].
#[must_use]
pub fn to_log_config(cfg: &Config) -> LogConfig {
    let format = match cfg.logging.format.as_str() {
        "pretty" => LogFormat::Pretty,
        "json" => LogFormat::Json,
        "full" => LogFormat::Full,
        _ => LogFormat::Compact,
    };

    let mut log_config = LogConfig::new(&cfg.logging.level).with_format(format);

    for directive in &cfg.logging.directives {
        log_config = log_config.with_directive(directive);
    }

    log_config
}

/// Convert config to [`WorkspaceConfig`].
pub fn to_workspace_config(cfg: &Config, workspace_root: &Path) -> WorkspaceConfig {
    let mode = match cfg.workspace.mode.as_str() {
        "guided" => WorkspaceMode::Guided,
        "autonomous" => WorkspaceMode::Autonomous,
        _ => WorkspaceMode::Safe,
    };

    let escape_policy = match cfg.workspace.escape_policy.as_str() {
        "deny" => EscapePolicy::Deny,
        "allow" => EscapePolicy::Allow,
        _ => EscapePolicy::Ask,
    };

    let mut ws = WorkspaceConfig::new(workspace_root)
        .with_mode(mode)
        .with_escape_policy(escape_policy);

    // Override never_allow from config if specified.
    ws.never_allow = cfg
        .workspace
        .never_allow
        .iter()
        .map(PathBuf::from)
        .collect();

    // Add auto-allow paths.
    for path in &cfg.workspace.auto_allow_read {
        ws = ws.allow_read(PathBuf::from(path));
    }

    for path in &cfg.workspace.auto_allow_write {
        ws = ws.allow_write(PathBuf::from(path));
    }

    ws
}

/// Get the workspace cumulative budget limit from config.
///
/// Returns `None` (unlimited) if not configured.
#[must_use]
pub fn workspace_max_usd(cfg: &Config) -> Option<f64> {
    cfg.budget.workspace_max_usd
}

/// Get the pricing rates from config (input cost per 1k tokens, output cost
/// per 1k tokens).
#[must_use]
pub fn pricing_rates(cfg: &Config) -> (f64, f64) {
    let input_per_1k = cfg.model.pricing.input_per_million / 1000.0;
    let output_per_1k = cfg.model.pricing.output_per_million / 1000.0;
    (input_per_1k, output_per_1k)
}

#[cfg(test)]
mod tests {
    use astrid_config::ServerSection;

    use super::*;

    #[test]
    fn convert_restart_policy_never() {
        let result = convert_restart_policy(&RestartPolicyConfig::Never);
        assert_eq!(result, RestartPolicy::Never);
    }

    #[test]
    fn convert_restart_policy_always() {
        let result = convert_restart_policy(&RestartPolicyConfig::Always);
        assert_eq!(result, RestartPolicy::Always);
    }

    #[test]
    fn convert_restart_policy_on_failure() {
        let result = convert_restart_policy(&RestartPolicyConfig::OnFailure { max_retries: 10 });
        assert_eq!(result, RestartPolicy::OnFailure { max_retries: 10 });
    }

    #[test]
    fn to_servers_config_wires_restart_policy_always() {
        let mut cfg = Config::default();
        cfg.servers.insert(
            "myserver".to_string(),
            ServerSection {
                command: Some("echo".to_string()),
                restart_policy: RestartPolicyConfig::Always,
                ..ServerSection::default()
            },
        );

        let servers = to_servers_config(&cfg);
        assert_eq!(
            servers.servers["myserver"].restart_policy,
            RestartPolicy::Always
        );
    }

    #[test]
    fn to_servers_config_wires_restart_policy_on_failure() {
        let mut cfg = Config::default();
        cfg.servers.insert(
            "myserver".to_string(),
            ServerSection {
                command: Some("echo".to_string()),
                restart_policy: RestartPolicyConfig::OnFailure { max_retries: 5 },
                ..ServerSection::default()
            },
        );

        let servers = to_servers_config(&cfg);
        assert_eq!(
            servers.servers["myserver"].restart_policy,
            RestartPolicy::OnFailure { max_retries: 5 }
        );
    }

    #[test]
    fn to_runtime_config_spark_seed_from_config() {
        let mut cfg = Config::default();
        cfg.spark.callsign = "Stellar".to_string();
        cfg.spark.class = "navigator".to_string();

        let rt = to_runtime_config(&cfg, Path::new("/tmp/test"));
        let seed = rt.spark_seed.expect("spark_seed should be Some");
        assert_eq!(seed.callsign, "Stellar");
        assert_eq!(seed.class, "navigator");
    }

    #[test]
    fn to_runtime_config_empty_spark_is_none() {
        let cfg = Config::default();
        let rt = to_runtime_config(&cfg, Path::new("/tmp/test"));
        assert!(rt.spark_seed.is_none());
    }

    /// Verify that `SparkSection` (config) and `SparkConfig` (tools) produce the
    /// same JSON keys when serialized from defaults. If someone adds a field
    /// to one but not the other, this test catches the mismatch.
    ///
    /// NOTE: `astrid_gateway::config::SparkConfig` is a third mirror type.
    /// The gateway `config_bridge` has its own parity test for that type.
    /// The canonical field set is defined by `astrid_config::SparkSection`.
    #[test]
    fn spark_section_and_config_have_matching_fields() {
        let section_json = serde_json::to_value(astrid_config::SparkSection::default()).unwrap();
        let config_json = serde_json::to_value(SparkConfig::default()).unwrap();

        let section_keys: std::collections::BTreeSet<String> =
            section_json.as_object().unwrap().keys().cloned().collect();
        let config_keys: std::collections::BTreeSet<String> =
            config_json.as_object().unwrap().keys().cloned().collect();

        assert_eq!(
            section_keys, config_keys,
            "SparkSection and SparkConfig have divergent field sets"
        );
    }

    #[test]
    fn to_servers_config_default_restart_policy_is_never() {
        let mut cfg = Config::default();
        cfg.servers.insert(
            "myserver".to_string(),
            ServerSection {
                command: Some("echo".to_string()),
                ..ServerSection::default()
            },
        );

        let servers = to_servers_config(&cfg);
        assert_eq!(
            servers.servers["myserver"].restart_policy,
            RestartPolicy::Never
        );
    }
}
