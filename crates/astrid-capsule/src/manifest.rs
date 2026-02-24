//! Capsule manifest types.
//!
//! A capsule manifest (`Capsule.toml`) describes a capsule's identity, entry point,
//! required capabilities, integrations, and configuration settings. Manifests are
//! loaded from disk during capsule discovery.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use astrid_core::ConnectorProfile;
use astrid_core::identity::FrontendType;

/// A capsule manifest loaded from `Capsule.toml`.
///
/// Describes everything the runtime needs to know about a capsule before
/// loading it: identity, component entry point, capability requirements,
/// settings, and OS integrations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleManifest {
    /// The package definition including name and version.
    pub package: PackageDef,
    /// The WASM or OpenClaw entry point definition.
    pub component: Option<ComponentDef>,
    /// Dependencies on other capsules.
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    /// Capabilities requested by this capsule.
    #[serde(default)]
    pub capabilities: CapabilitiesDef,
    /// Environment variables configurable by the user during docking.
    #[serde(default)]
    pub env: HashMap<String, EnvDef>,
    /// Context files to inject.
    #[serde(default, rename = "context_file")]
    pub context_files: Vec<ContextFileDef>,
    /// Commands this capsule provides.
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandDef>,
    /// MCP servers this capsule exposes.
    #[serde(default, rename = "mcp_server")]
    pub mcp_servers: Vec<McpServerDef>,
    /// Skills this capsule provides.
    #[serde(default, rename = "skill")]
    pub skills: Vec<SkillDef>,
    /// Uplinks this capsule provides (e.g. Telegram, CLI frontend).
    #[serde(default, rename = "uplink")]
    pub uplinks: Vec<UplinkDef>,
    /// LLM Providers (Agents) this capsule exposes to the OS.
    #[serde(default, rename = "llm_provider")]
    pub llm_providers: Vec<LlmProviderDef>,
    /// Interceptors (eBPF-style hooks) this capsule registers.
    #[serde(default, rename = "interceptor")]
    pub interceptors: Vec<InterceptorDef>,
    /// Scheduled background tasks (cron jobs) this capsule provides.
    #[serde(default, rename = "cron")]
    pub cron_jobs: Vec<CronDef>,
    /// Native tools this capsule provides to the LLM agent.
    #[serde(default, rename = "tool")]
    pub tools: Vec<ToolDef>,
}

/// Package identity metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDef {
    /// The capsule's unique name.
    pub name: String,
    /// The semantic version.
    pub version: String,
    /// Optional description of the capsule.
    pub description: Option<String>,
    /// Optional authors of the capsule.
    #[serde(default)]
    pub authors: Vec<String>,
    /// Optional repository URL.
    pub repository: Option<String>,
    /// Optional homepage URL.
    pub homepage: Option<String>,
    /// Optional documentation URL.
    pub documentation: Option<String>,
    /// Optional license identifier (e.g., "MIT OR Apache-2.0").
    pub license: Option<String>,
    /// Optional path to a non-standard license file.
    #[serde(rename = "license-file")]
    pub license_file: Option<PathBuf>,
    /// Optional path to a README file.
    pub readme: Option<PathBuf>,
    /// Search keywords (up to 5).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Registry categories (up to 5).
    #[serde(default)]
    pub categories: Vec<String>,
    /// The required version of the Astrid OS (e.g., ">=0.1.0").
    #[serde(rename = "astrid-version")]
    pub astrid_version: Option<String>,
    /// Whether this capsule is allowed to be published to a registry (defaults to true).
    pub publish: Option<bool>,
    /// Glob patterns of files to explicitly include when packing the capsule.
    pub include: Option<Vec<String>>,
    /// Glob patterns of files to exclude when packing the capsule.
    pub exclude: Option<Vec<String>>,
    /// A catch-all table for custom, tool-specific metadata.
    pub metadata: Option<serde_json::Value>,
}

/// Defines the main executable component of the capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentDef {
    /// Path to the WASM file or OpenClaw script.
    pub entrypoint: PathBuf,
    /// Expected hash for security verification.
    pub hash: Option<String>,
}

/// A collection of capabilities the capsule requests from the OS.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilitiesDef {
    /// Network domains the capsule wants to access.
    #[serde(default)]
    pub net: Vec<String>,
    /// Scoped KV store access requests.
    #[serde(default)]
    pub kv: Vec<String>,
    /// VFS read paths.
    #[serde(default)]
    pub fs_read: Vec<String>,
    /// VFS write paths.
    #[serde(default)]
    pub fs_write: Vec<String>,
    /// Legacy host process executions (the "Airlock Override").
    #[serde(default)]
    pub host_process: Vec<String>,
}

/// An environment variable required by the capsule.
///
/// These are securely elicited from the user during `capsule install` (docking).
/// This prevents developers from shipping hardcoded API keys in their manifests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvDef {
    /// The type of the environment variable (e.g. "secret", "string").
    #[serde(rename = "type")]
    pub env_type: String,
    /// The specific prompt or question to ask the user when eliciting this value.
    pub request: Option<String>,
    /// The human-readable description.
    pub description: Option<String>,
    /// An optional default value.
    pub default: Option<serde_json::Value>,
}

/// A context file provided by the capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFileDef {
    /// The name of the context block.
    pub name: String,
    /// The path to the context file.
    pub file: PathBuf,
}

/// A command provided by the capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDef {
    /// The slash-command trigger.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Path to the declarative command TOML (if static).
    pub file: Option<PathBuf>,
}

/// An MCP server provided by the capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDef {
    /// Unique ID for the MCP server.
    pub id: String,
    /// Optional description.
    pub description: Option<String>,
    /// Server type: "wasm-ipc", "stdio", "openclaw".
    #[serde(rename = "type")]
    pub server_type: Option<String>,
    /// The host command (if type = stdio).
    pub command: Option<String>,
    /// The host arguments (if type = stdio).
    #[serde(default)]
    pub args: Vec<String>,
}

/// A skill provided by the capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDef {
    /// Name of the skill.
    pub name: String,
    /// Description of what the skill provides.
    pub description: Option<String>,
    /// Path to the skill file.
    pub file: PathBuf,
}

/// An uplink provided by the capsule (e.g., Telegram, CLI).
///
/// This allows the LLM agent to route messages out to a specific frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UplinkDef {
    /// Unique name of the uplink.
    pub name: String,
    /// The platform identifier (e.g., "telegram", "cli").
    pub platform: FrontendType,
    /// The interaction profile (e.g., "human", "bridge").
    pub profile: ConnectorProfile,
}

/// An LLM Provider (Agent) exposed by the capsule.
///
/// This allows a capsule to act as the "brain" for a session, receiving prompts
/// from the OS Event Bus and streaming text/tool-calls back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderDef {
    /// Unique identifier for this provider/model (e.g., "claude-3-5-sonnet").
    pub id: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Capabilities this model supports (e.g., "text", "vision", "tools").
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A tool provided by the capsule to the LLM agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// Name of the tool.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON schema for the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// An eBPF-style interceptor hook provided by the capsule.
///
/// This allows the OS to synchronously route specific lifecycle events
/// (like `BeforeToolCall` or `BeforeAgentResponse`) through this capsule
/// for filtering or policy enforcement without introspecting the WASM binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptorDef {
    /// The specific OS event to intercept (e.g., "BeforeToolCall").
    pub event: String,
}

/// A scheduled background task (cron job) provided by the capsule.
///
/// Allows the OS scheduler to trigger capsule logic at specific intervals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronDef {
    /// The name of the scheduled task.
    pub name: String,
    /// The cron expression schedule (e.g., "0 0 * * *").
    pub schedule: String,
    /// The action or topic to trigger when the schedule fires.
    pub action: String,
}
