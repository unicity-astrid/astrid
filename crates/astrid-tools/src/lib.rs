#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
//! Built-in coding tools for the Astrid agent runtime.
//!
//! Provides 9 tools as direct Rust function calls (not MCP) for the hot-path
//! coding operations: read, write, edit, search, execute, and identity.

mod bash;
mod edit_file;
mod glob;
mod grep;
mod instructions;
mod list_directory;
mod read_file;
pub mod spark;
mod spark_tool;
mod subagent_spawner;
mod system_prompt;
mod task;
mod write_file;

pub use bash::BashTool;
pub use edit_file::EditFileTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use instructions::load_project_instructions;
pub use list_directory::ListDirectoryTool;
pub use read_file::ReadFileTool;
pub use spark::SparkConfig;
pub use spark_tool::SparkTool;
pub use subagent_spawner::{SubAgentRequest, SubAgentResult, SubAgentSpawner};
pub use system_prompt::build_system_prompt;
pub use task::TaskTool;
pub use write_file::WriteFileTool;

use astrid_llm::LlmToolDefinition;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Maximum output size in characters before truncation.
const MAX_OUTPUT_CHARS: usize = 30_000;

/// A built-in tool that executes directly in-process.
#[async_trait::async_trait]
pub trait BuiltinTool: Send + Sync {
    /// Tool name (no colons — distinguishes from MCP "server:tool" format).
    fn name(&self) -> &'static str;

    /// Human-readable description for the LLM.
    fn description(&self) -> &'static str;

    /// JSON schema for tool input parameters.
    fn input_schema(&self) -> Value;

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult;
}

/// Shared context available to all built-in tools.
pub struct ToolContext {
    /// Workspace root directory.
    pub workspace_root: PathBuf,
    /// Current working directory (persists across bash invocations).
    pub cwd: Arc<RwLock<PathBuf>>,
    /// Path to the living spark file (`~/.astrid/spark.toml`).
    pub spark_file: Option<PathBuf>,
    /// Sub-agent spawner (set by runtime before each turn, cleared after).
    subagent_spawner: RwLock<Option<Arc<dyn SubAgentSpawner>>>,
}

impl ToolContext {
    /// Create a new tool context.
    #[must_use]
    pub fn new(workspace_root: PathBuf, spark_file: Option<PathBuf>) -> Self {
        let cwd = Arc::new(RwLock::new(workspace_root.clone()));
        Self {
            workspace_root,
            cwd,
            spark_file,
            subagent_spawner: RwLock::new(None),
        }
    }

    /// Create a per-turn tool context that shares the `cwd` with other turns
    /// but has its own independent spawner slot.
    ///
    /// This prevents concurrent sessions from racing on the spawner field
    /// while still sharing the working directory state.
    #[must_use]
    pub fn with_shared_cwd(
        workspace_root: PathBuf,
        cwd: Arc<RwLock<PathBuf>>,
        spark_file: Option<PathBuf>,
    ) -> Self {
        Self {
            workspace_root,
            cwd,
            spark_file,
            subagent_spawner: RwLock::new(None),
        }
    }

    /// Set the sub-agent spawner (called by runtime at turn start).
    pub async fn set_subagent_spawner(&self, spawner: Option<Arc<dyn SubAgentSpawner>>) {
        *self.subagent_spawner.write().await = spawner;
    }

    /// Get the sub-agent spawner (called by `TaskTool`).
    pub async fn subagent_spawner(&self) -> Option<Arc<dyn SubAgentSpawner>> {
        self.subagent_spawner.read().await.clone()
    }
}

/// Tool execution errors.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid arguments.
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    /// Execution failed.
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    /// Path not found.
    #[error("Path not found: {0}")]
    PathNotFound(String),

    /// Timeout.
    #[error("Timeout after {0}ms")]
    Timeout(u64),

    /// Other error.
    #[error("{0}")]
    Other(String),
}

/// Result type for tool execution.
pub type ToolResult = Result<String, ToolError>;

/// Registry of built-in tools for lookup and LLM definition export.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn BuiltinTool>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Create a registry with all default tools registered.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(ReadFileTool));
        registry.register(Box::new(WriteFileTool));
        registry.register(Box::new(EditFileTool));
        registry.register(Box::new(GlobTool));
        registry.register(Box::new(GrepTool));
        registry.register(Box::new(BashTool));
        registry.register(Box::new(ListDirectoryTool));
        registry.register(Box::new(TaskTool));
        registry.register(Box::new(SparkTool));
        registry
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Box<dyn BuiltinTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn BuiltinTool> {
        self.tools.get(name).map(AsRef::as_ref)
    }

    /// Check if a name refers to a built-in tool (no colon = built-in).
    #[must_use]
    pub fn is_builtin(name: &str) -> bool {
        !name.contains(':')
    }

    /// Export all tool definitions for the LLM.
    #[must_use]
    pub fn all_definitions(&self) -> Vec<LlmToolDefinition> {
        self.tools
            .values()
            .map(|t| {
                LlmToolDefinition::new(t.name())
                    .with_description(t.description())
                    .with_schema(t.input_schema())
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate output to stay within LLM context limits.
///
/// If `output` exceeds [`MAX_OUTPUT_CHARS`], it is truncated and a notice is appended.
#[must_use]
pub fn truncate_output(output: String) -> String {
    if output.len() <= MAX_OUTPUT_CHARS {
        return output;
    }
    let mut truncated = output[..MAX_OUTPUT_CHARS].to_string();
    truncated.push_str("\n\n... (output truncated — exceeded 30000 character limit)");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_builtin() {
        assert!(ToolRegistry::is_builtin("read_file"));
        assert!(ToolRegistry::is_builtin("bash"));
        assert!(!ToolRegistry::is_builtin("filesystem:read_file"));
    }

    #[test]
    fn test_registry_with_defaults() {
        let registry = ToolRegistry::with_defaults();
        assert!(registry.get("read_file").is_some());
        assert!(registry.get("write_file").is_some());
        assert!(registry.get("edit_file").is_some());
        assert!(registry.get("glob").is_some());
        assert!(registry.get("grep").is_some());
        assert!(registry.get("bash").is_some());
        assert!(registry.get("list_directory").is_some());
        assert!(registry.get("task").is_some());
        assert!(registry.get("spark").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_all_definitions() {
        let registry = ToolRegistry::with_defaults();
        let defs = registry.all_definitions();
        assert_eq!(defs.len(), 9);
        for def in &defs {
            assert!(!def.name.contains(':'));
            assert!(def.description.is_some());
        }
    }

    #[test]
    fn test_truncate_output_small() {
        let small = "hello".to_string();
        assert_eq!(truncate_output(small.clone()), small);
    }

    #[test]
    fn test_truncate_output_large() {
        let large = "x".repeat(40_000);
        let result = truncate_output(large);
        assert!(result.len() < 40_000);
        assert!(result.contains("output truncated"));
    }
}
