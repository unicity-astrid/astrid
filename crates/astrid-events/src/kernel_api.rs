use serde::{Deserialize, Serialize};

/// Management API requests directed at the core daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum KernelRequest {
    /// Request to install a capsule from a local or remote path.
    InstallCapsule {
        /// The path or URL to the `.capsule` archive.
        source: String,
        /// True if this should be installed locally in the workspace.
        workspace: bool,
    },
    /// Request to approve a capability grant (usually following an `ApprovalNeeded` response).
    ApproveCapability {
        /// The unique ID of the request being approved.
        request_id: String,
        /// Cryptographic signature proving Root Identity authorization.
        signature: String,
    },
    /// Request the list of currently loaded capsules.
    ListCapsules,
    /// Reload all capsules from the file system.
    ReloadCapsules,
    /// Request the list of globally registered slash commands.
    GetCommands,
    /// Request metadata about loaded capsules (manifests, providers, interceptors).
    /// The kernel's equivalent of `/proc` — exposing process table info.
    GetCapsuleMetadata,
}

/// Management API responses from the core daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "data")]
pub enum KernelResponse {
    /// The request succeeded.
    Success(serde_json::Value),
    /// A list of available slash commands across all capsules.
    Commands(Vec<CommandInfo>),
    /// Metadata about loaded capsules.
    CapsuleMetadata(Vec<CapsuleMetadataEntry>),
    /// The request failed.
    Error(String),
    /// The request requires user capability approval before it can proceed.
    ApprovalRequired {
        /// Unique ID for this specific action request.
        request_id: String,
        /// Description of what is being requested.
        description: String,
        /// The specific capabilities required (e.g. `["host_process", "fs_write"]`).
        capabilities: Vec<String>,
    },
}

/// Metadata entry for a loaded capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleMetadataEntry {
    /// The capsule's unique name.
    pub name: String,
    /// LLM providers declared by this capsule.
    pub llm_providers: Vec<LlmProviderInfo>,
    /// Interceptor event patterns declared by this capsule.
    pub interceptor_events: Vec<String>,
}

/// Information about an available LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderInfo {
    /// The model ID (e.g. `"claude-3-5-sonnet-20241022"`).
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Capabilities (e.g. `["text", "vision", "tools"]`).
    pub capabilities: Vec<String>,
}

/// Information about a registered slash command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    /// The slash command trigger (e.g. `/git`).
    pub name: String,
    /// A brief description of what the command does.
    pub description: String,
    /// The capsule that provides this command.
    pub provider_capsule: String,
}
