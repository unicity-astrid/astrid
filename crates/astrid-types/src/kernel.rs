//! Kernel management API request and response types.

use serde::{Deserialize, Serialize};

/// The well-known system session UUID string used by the background daemon.
///
/// All kernel-internal IPC messages are published with this `source_id`.
/// WASM capsules that verify message provenance should compare against
/// this constant. Mirrors `astrid_core::SessionId::SYSTEM`.
pub const SYSTEM_SESSION_UUID: &str = "00000000-0000-0000-0000-000000000000";

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
    /// Request the daemon to shut down gracefully.
    Shutdown {
        /// Optional reason for shutdown.
        reason: Option<String>,
    },
    /// Request daemon status information.
    GetStatus,
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
    /// Daemon status information.
    Status(DaemonStatus),
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

/// Daemon runtime status information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    /// Process ID of the daemon.
    pub pid: u32,
    /// Daemon uptime in seconds.
    pub uptime_secs: u64,
    /// Daemon version string.
    pub version: String,
    /// Whether the daemon is running in ephemeral mode.
    pub ephemeral: bool,
    /// Number of currently connected clients.
    pub connected_clients: u32,
    /// Names of loaded capsules.
    pub loaded_capsules: Vec<String>,
}

/// Metadata entry for a loaded capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleMetadataEntry {
    /// The capsule's unique name.
    pub name: String,
    /// Interceptor event patterns declared by this capsule.
    pub interceptor_events: Vec<String>,
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
