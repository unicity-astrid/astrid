//! Capsule manifest types.
//!
//! A capsule manifest (`Capsule.toml`) describes a capsule's identity, entry point,
//! required capabilities, integrations, and configuration settings. Manifests are
//! loaded from disk during capsule discovery.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use astrid_core::UplinkProfile;
/// A capsule manifest loaded from `Capsule.toml`.
///
/// Describes everything the runtime needs to know about a capsule before
/// loading it: identity, component entry point, capability requirements,
/// settings, and OS integrations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleManifest {
    /// The package definition including name and version.
    pub package: PackageDef,
    /// The WASM components provided by this capsule.
    #[serde(default, rename = "component")]
    pub components: Vec<ComponentDef>,
    /// Capability-based dependency declarations for boot ordering.
    #[serde(default)]
    pub dependencies: DependenciesDef,
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
    /// Uplinks this capsule provides (e.g. Telegram, CLI).
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
    /// Cached effective provides (computed lazily on first `effective_provides()` call).
    ///
    /// **Do not pre-populate.** Always initialize as `OnceLock::new()` in struct literals.
    /// After `Clone`, the cache carries over the already-initialized value. Mutating
    /// fields of the clone after `effective_provides()` was called on either the
    /// original or the clone will return stale data.
    #[serde(skip)]
    #[doc(hidden)]
    pub effective_provides_cache: OnceLock<Vec<String>>,
}

impl CapsuleManifest {
    /// Compute the effective set of provided capabilities.
    ///
    /// If `dependencies.provides` is explicitly non-empty, returns it directly.
    /// Otherwise, auto-derives capabilities from `ipc_publish` topics, tools,
    /// LLM providers, and uplinks using typed prefixes (`topic:`, `tool:`,
    /// `llm:`, `uplink:`).
    #[must_use]
    pub fn effective_provides(&self) -> &[String] {
        self.effective_provides_cache.get_or_init(|| {
            if !self.dependencies.provides.is_empty() {
                return self.dependencies.provides.clone();
            }
            let mut caps = Vec::new();
            for topic in &self.capabilities.ipc_publish {
                caps.push(format!("topic:{topic}"));
            }
            for tool in &self.tools {
                caps.push(format!("tool:{}", tool.name));
            }
            for provider in &self.llm_providers {
                caps.push(format!("llm:{}", provider.id));
            }
            for uplink in &self.uplinks {
                caps.push(format!("uplink:{}", uplink.name));
            }
            caps
        })
    }
}

/// Capability-based dependency declarations for boot ordering.
///
/// Capsules declare what capabilities they `provide` to the system and what
/// they `require` to be present before booting. Capabilities use typed
/// prefixes:
///
/// - `topic:llm.stream.anthropic` - IPC topic
/// - `tool:run_shell_command` - tool availability
/// - `llm:claude-3-5-sonnet` - LLM provider
/// - `uplink:cli` - uplink/frontend
///
/// Wildcards (`*`) match a single dot-separated segment:
/// `topic:llm.stream.*` matches `topic:llm.stream.anthropic`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DependenciesDef {
    /// Capabilities this capsule provides to the system.
    ///
    /// Auto-derived from `ipc_publish`, `tools`, `llm_providers`, and
    /// `uplinks` if not explicitly declared.
    #[serde(default)]
    pub provides: Vec<String>,

    /// Capabilities that MUST be provided by another loaded capsule
    /// before this capsule boots. Any single provider satisfying a
    /// requirement is sufficient (any-satisfies semantic).
    #[serde(default)]
    pub requires: Vec<String>,
}

impl DependenciesDef {
    /// Returns `true` if no capabilities are declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.provides.is_empty() && self.requires.is_empty()
    }
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

/// Defines an executable or library component within the capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentDef {
    /// Unique identifier for this component within the capsule.
    #[serde(default)]
    pub id: String,
    /// Path to the WASM file.
    #[serde(rename = "file", alias = "entrypoint")]
    pub path: PathBuf,
    /// Expected hash for security verification.
    pub hash: Option<String>,
    /// Type of component: "executable" (default) or "library".
    #[serde(default)]
    pub r#type: String,
    /// List of component IDs this component dynamically links to.
    #[serde(default)]
    pub link: Vec<String>,
    /// Capabilities specifically requested by this component.
    #[serde(default)]
    pub capabilities: Option<CapabilitiesDef>,
}

/// A collection of capabilities the capsule requests from the OS.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilitiesDef {
    /// Whether the capsule acts as a long-lived uplink/daemon (e.g. the CLI proxy).
    /// When true, the WASM execution timeout is disabled.
    #[serde(default)]
    pub uplink: bool,
    /// Network domains the capsule wants to access.
    #[serde(default)]
    pub net: Vec<String>,
    /// Scoped KV store access requests.
    /// Note: KV access is inherently scoped per-capsule at runtime,
    /// so this field is currently not enforced via a security gate, but
    /// is present for future cross-capsule KV request declarations.
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
    /// Unix/TCP socket bind addresses the capsule requires.
    #[serde(default)]
    pub net_bind: Vec<String>,
    /// IPC topic patterns this capsule is allowed to publish to.
    ///
    /// Supports exact matches and `*` wildcards per segment
    /// (e.g. `registry.*`, `llm.stream.anthropic`).
    /// An empty list means the capsule may NOT publish to any topic
    /// (fail-closed). Capsules must explicitly declare at least one
    /// pattern to be allowed to publish.
    #[serde(default)]
    pub ipc_publish: Vec<String>,
    /// IPC topic patterns this capsule is allowed to subscribe to.
    ///
    /// Uses the same matching semantics as `ipc_publish`: exact matches
    /// and `*` wildcards per segment, with segment counts required to
    /// match. An empty list means the capsule may NOT subscribe to any
    /// topic (fail-closed).
    ///
    /// Note: the ACL gates the subscription *pattern string*, not
    /// individual messages. The ACL uses `topic_matches` semantics
    /// (single-segment `*`, equal segment count required), but the
    /// `EventBus` delivers events using `EventReceiver::matches` where
    /// a trailing `*` matches one or more segments. This means
    /// `ipc_subscribe = ["foo.v1.*"]` authorizes subscribing to the
    /// pattern `"foo.v1.*"`, which the EventBus will use to deliver
    /// events at any depth under `foo.v1.` - not just single-segment.
    /// Per-message ACL checking would be O(n) per delivery and is
    /// architecturally wrong for a broadcast bus.
    #[serde(default)]
    pub ipc_subscribe: Vec<String>,
    /// Identity operations this capsule is allowed to perform.
    ///
    /// Valid values: `"resolve"` (read-only lookups), `"link"` (create/delete
    /// links, list links), `"admin"` (create users). The hierarchy is
    /// `admin > link > resolve` - higher levels imply all lower levels.
    ///
    /// An empty list means NO identity access (fail-closed).
    #[serde(default)]
    pub identity: Vec<String>,
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
    /// Valid choices for enum fields.
    #[serde(default)]
    pub enum_values: Vec<String>,
    /// Placeholder hint text shown in an empty input field (e.g. `"sk-..."`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
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
/// This allows the LLM agent to route messages out to a specific platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UplinkDef {
    /// Unique name of the uplink.
    pub name: String,
    /// The platform identifier (e.g., "telegram", "cli").
    pub platform: String,
    /// The interaction profile (e.g., "human", "bridge").
    pub profile: UplinkProfile,
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

/// An event interceptor registered by the capsule.
///
/// Maps an IPC event topic pattern to a named action (WASM export handler).
/// The kernel's event dispatcher matches incoming IPC events against the
/// `event` pattern and invokes `astrid_hook_trigger` with the `action` name
/// and the event payload.
///
/// Topic patterns support single-segment wildcards: `tool.execute.*.result`
/// matches `tool.execute.search.result` but not `tool.execute.result`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptorDef {
    /// IPC topic pattern to match (e.g., `user.prompt`, `tool.execute.*.result`).
    pub event: String,
    /// Name of the handler function inside the WASM guest
    /// (must match an `#[astrid::interceptor("...")]` annotation).
    pub action: String,
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
