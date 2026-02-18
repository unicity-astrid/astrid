//! Runtime configuration types and defaults.

use astrid_tools::SparkConfig;
use astrid_workspace::WorkspaceConfig;
use std::path::PathBuf;

use crate::subagent_executor::DEFAULT_SUBAGENT_TIMEOUT;

/// Default maximum context tokens (100k).
pub(super) const DEFAULT_MAX_CONTEXT_TOKENS: usize = 100_000;
/// Default number of recent messages to keep when summarizing.
pub(super) const DEFAULT_KEEP_RECENT_COUNT: usize = 10;

/// Default maximum concurrent sub-agents.
pub(super) const DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 4;
/// Default maximum sub-agent nesting depth.
pub(super) const DEFAULT_MAX_SUBAGENT_DEPTH: usize = 3;

/// Configuration for the agent runtime.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Maximum context tokens.
    pub max_context_tokens: usize,
    /// System prompt.
    pub system_prompt: String,
    /// Whether to auto-summarize on context overflow.
    pub auto_summarize: bool,
    /// Number of recent messages to keep when summarizing.
    pub keep_recent_count: usize,
    /// Workspace configuration for operational boundaries.
    pub workspace: WorkspaceConfig,
    /// Maximum concurrent sub-agents.
    pub max_concurrent_subagents: usize,
    /// Maximum sub-agent nesting depth.
    pub max_subagent_depth: usize,
    /// Default sub-agent timeout.
    pub default_subagent_timeout: std::time::Duration,
    /// Static spark seed from `[spark]` in config (fallback when spark.toml missing).
    pub spark_seed: Option<SparkConfig>,
    /// Path to the living spark file (`~/.astrid/spark.toml`).
    ///
    /// **Note:** When a spark identity is configured (either from this file or
    /// from `spark_seed`), the spark preamble is prepended to the system prompt
    /// on every LLM call for non-sub-agent sessions. If `system_prompt` is set
    /// to a custom value, the spark preamble is still prepended. Sub-agent
    /// sessions skip spark injection to avoid double-identity conflicts.
    pub spark_file: Option<PathBuf>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            max_context_tokens: DEFAULT_MAX_CONTEXT_TOKENS,
            system_prompt: String::new(),
            auto_summarize: true,
            keep_recent_count: DEFAULT_KEEP_RECENT_COUNT,
            workspace: WorkspaceConfig::new(workspace_root),
            max_concurrent_subagents: DEFAULT_MAX_CONCURRENT_SUBAGENTS,
            max_subagent_depth: DEFAULT_MAX_SUBAGENT_DEPTH,
            default_subagent_timeout: DEFAULT_SUBAGENT_TIMEOUT,
            spark_seed: None,
            spark_file: None,
        }
    }
}
