//! Capsule manifest types.
//!
//! A capsule manifest (`Capsule.toml`) describes a capsule's identity, entry point,
//! required capabilities, integrations, and configuration settings. Manifests are
//! loaded from disk during capsule discovery.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

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
    /// Namespaced interface imports — what this capsule needs from others.
    ///
    /// Outer key = namespace (e.g. `"astrid"`), inner key = interface name
    /// (e.g. `"session"`), value = version requirement and optional flag.
    #[serde(default)]
    pub imports: ImportsMap,
    /// Namespaced interface exports — what this capsule provides.
    ///
    /// Outer key = namespace, inner key = interface name, value = exact version.
    #[serde(default)]
    pub exports: ExportsMap,
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
    /// Interceptors (eBPF-style hooks) this capsule registers.
    #[serde(default, rename = "interceptor")]
    pub interceptors: Vec<InterceptorDef>,
    /// Topic API declarations describing the payload shape of IPC topics.
    #[serde(default, rename = "topic")]
    pub topics: Vec<TopicDef>,
}

impl CapsuleManifest {
    /// Returns `true` if this capsule declares any imports.
    #[must_use]
    pub fn has_imports(&self) -> bool {
        self.imports.values().any(|ns| !ns.is_empty())
    }

    /// Returns `true` if this capsule declares any exports.
    #[must_use]
    pub fn has_exports(&self) -> bool {
        self.exports.values().any(|ns| !ns.is_empty())
    }

    /// Iterate all exported interfaces as `(namespace, name, version)` triples.
    pub fn export_triples(&self) -> impl Iterator<Item = (&str, &str, &semver::Version)> {
        self.exports.iter().flat_map(|(ns, ifaces)| {
            ifaces
                .iter()
                .map(move |(name, def)| (ns.as_str(), name.as_str(), &def.version))
        })
    }

    /// Iterate all imported interfaces as `(namespace, name, version_req, optional)` tuples.
    pub fn import_tuples(&self) -> impl Iterator<Item = (&str, &str, &semver::VersionReq, bool)> {
        self.imports.iter().flat_map(|(ns, ifaces)| {
            ifaces
                .iter()
                .map(move |(name, def)| (ns.as_str(), name.as_str(), &def.version, def.optional))
        })
    }
}

/// Namespaced interface imports. Outer key = namespace, inner key = interface name.
pub type ImportsMap = HashMap<String, HashMap<String, ImportDef>>;

/// Namespaced interface exports. Outer key = namespace, inner key = interface name.
pub type ExportsMap = HashMap<String, HashMap<String, ExportDef>>;

/// An imported interface — version requirement with optional flag.
///
/// Deserializes from either a version string (`"^1.0"`) or a table
/// (`{ version = "^1.0", optional = true }`).
#[derive(Debug, Clone, Serialize)]
pub struct ImportDef {
    /// Semver version requirement (e.g. `^1.0`, `>=1.0, <2.0`, `*`).
    pub version: semver::VersionReq,
    /// If `true`, the capsule boots even if no provider is loaded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

impl<'de> Deserialize<'de> for ImportDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Short(String),
            Full {
                version: String,
                #[serde(default)]
                optional: bool,
            },
        }
        let raw = Raw::deserialize(deserializer)?;
        let (version_str, optional) = match raw {
            Raw::Short(s) => (s, false),
            Raw::Full { version, optional } => (version, optional),
        };
        let version = semver::VersionReq::parse(&version_str).map_err(|e| {
            serde::de::Error::custom(format!("invalid semver requirement '{version_str}': {e}"))
        })?;
        Ok(Self { version, optional })
    }
}

/// An exported interface — exact version declaration.
///
/// Deserializes from either a version string (`"1.0.0"`) or a table
/// (`{ version = "1.0.0" }`).
#[derive(Debug, Clone, Serialize)]
pub struct ExportDef {
    /// Exact semver version this capsule provides.
    pub version: semver::Version,
}

impl<'de> Deserialize<'de> for ExportDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Short(String),
            Full { version: String },
        }
        let raw = Raw::deserialize(deserializer)?;
        let version_str = match raw {
            Raw::Short(s) => s,
            Raw::Full { version } => version,
        };
        let version = semver::Version::parse(&version_str).map_err(|e| {
            serde::de::Error::custom(format!("invalid semver version '{version_str}': {e}"))
        })?;
        Ok(Self { version })
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
    /// Whether the capsule may override or modify the system prompt via the
    /// prompt builder's hook pipeline.
    ///
    /// When `false` (default), hook responses from this capsule have their
    /// `systemPrompt`, `prependSystemContext`, and `appendSystemContext`
    /// fields stripped. Only `prependContext` (user-visible context) passes
    /// through.
    ///
    /// This is a critical security boundary: unprivileged capsules cannot
    /// inject arbitrary instructions into the LLM's system prompt.
    #[serde(default)]
    pub allow_prompt_injection: bool,
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
    /// Dispatch priority — lower values fire first. Default 100.
    /// Enables layered interception (e.g. input guard at 10 fires before
    /// react loop at 100).
    #[serde(default = "default_interceptor_priority")]
    pub priority: u32,
}

/// Default interceptor priority.
const fn default_interceptor_priority() -> u32 {
    100
}

/// Direction a capsule interacts with an IPC topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TopicDirection {
    /// The capsule publishes messages to this topic.
    Publish,
    /// The capsule subscribes to messages on this topic.
    Subscribe,
}

impl fmt::Display for TopicDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Publish => f.write_str("publish"),
            Self::Subscribe => f.write_str("subscribe"),
        }
    }
}

/// A topic API declaration describing the payload shape of an IPC topic.
///
/// Capsules declare each published or subscribed topic with an optional
/// JSON Schema file or a reference to a WIT record type. At install time,
/// the schema is baked into `meta.json` for tooling and A2UI consumption.
///
/// If both `schema` and `wit_type` are set, `wit_type` takes precedence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicDef {
    /// The concrete topic name (e.g. `"llm.v1.response.chunk.anthropic"`).
    /// Wildcards are not permitted; topic declarations must be concrete API contracts.
    pub name: String,
    /// Whether the capsule publishes or subscribes to this topic.
    pub direction: TopicDirection,
    /// Human-readable description of the topic's purpose.
    pub description: Option<String>,
    /// Path to a JSON Schema file (relative to the capsule directory).
    pub schema: Option<PathBuf>,
    /// Name of a WIT record type (kebab-case) defined in the capsule's `wit/` directory.
    /// At install time, the record is parsed from WIT and converted to JSON Schema
    /// with field descriptions from `///` doc comments.
    pub wit_type: Option<String>,
}
