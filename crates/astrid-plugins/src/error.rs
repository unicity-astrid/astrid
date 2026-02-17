//! Plugin error types.

use std::path::PathBuf;

use astrid_core::ConnectorId;

use crate::PluginId;

/// Errors from plugin operations.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The requested plugin was not found in the registry.
    #[error("plugin not found: {0}")]
    NotFound(PluginId),

    /// A plugin with this ID is already registered.
    #[error("plugin already registered: {0}")]
    AlreadyRegistered(PluginId),

    /// A connector with this ID is already registered.
    #[error("connector already registered: {0}")]
    ConnectorAlreadyRegistered(ConnectorId),

    /// The requested connector was not found in the registry.
    #[error("connector not found: {0}")]
    ConnectorNotFound(ConnectorId),

    /// Failed to parse a plugin manifest file.
    #[error("manifest parse error in {path}: {message}")]
    ManifestParseError {
        /// Path to the manifest file.
        path: PathBuf,
        /// Parse error message.
        message: String,
    },

    /// Plugin failed to load.
    #[error("plugin load failed: {plugin_id} - {message}")]
    LoadFailed {
        /// The plugin that failed to load.
        plugin_id: PluginId,
        /// Failure reason.
        message: String,
    },

    /// Plugin tool execution failed.
    #[error("plugin execution failed: {0}")]
    ExecutionFailed(String),

    /// The requested tool was not found.
    #[error("tool not found: {0}")]
    ToolNotFound(String),

    /// The plugin ID is invalid.
    #[error("invalid plugin id: {0}")]
    InvalidId(String),

    /// Storage operation failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// The MCP server for a plugin failed.
    #[error("MCP server failed for plugin {plugin_id}: {message}")]
    McpServerFailed {
        /// The plugin whose MCP server failed.
        plugin_id: PluginId,
        /// Failure reason.
        message: String,
    },

    /// An MCP client is required but was not provided.
    #[error("MCP client required for MCP plugin entry point")]
    McpClientRequired,

    /// The plugin entry point type is not supported by this factory.
    #[error("unsupported entry point type: {0}")]
    UnsupportedEntryPoint(String),

    /// Sandbox profile error.
    #[error("sandbox error: {0}")]
    SandboxError(String),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// WASM runtime error (Extism/Wasmtime).
    #[error("WASM error: {0}")]
    WasmError(String),

    /// npm registry API failure.
    #[error("registry error: {message}")]
    RegistryError {
        /// Description of the registry failure.
        message: String,
    },

    /// SHA-512 SRI integrity verification failed.
    #[error("integrity mismatch for {package}: expected {expected}")]
    IntegrityError {
        /// Package that failed verification.
        package: String,
        /// Expected SRI hash string.
        expected: String,
    },

    /// Tarball extraction failure.
    #[error("extraction error: {message}")]
    ExtractionError {
        /// Description of the extraction failure.
        message: String,
    },

    /// Unsafe entry type in archive (e.g. symlink, hardlink, device node).
    #[error("unsafe archive entry type '{entry_type}' at {path}")]
    UnsafeEntryType {
        /// The entry type that was rejected.
        entry_type: String,
        /// The path of the entry.
        path: String,
    },

    /// Path traversal detected in archive entry.
    #[error("path traversal detected: {path}")]
    PathTraversal {
        /// The offending path.
        path: String,
    },

    /// Tarball exceeds maximum allowed size.
    #[error("package too large: {size} bytes (limit: {limit} bytes)")]
    PackageTooLarge {
        /// Actual size in bytes.
        size: u64,
        /// Maximum allowed size in bytes.
        limit: u64,
    },

    /// Package is not an `OpenClaw` plugin (missing `openclaw.plugin.json`).
    #[error("not an OpenClaw plugin: missing openclaw.plugin.json")]
    NotOpenClawPlugin,

    /// Invalid npm package name.
    #[error("invalid package name '{name}': {reason}")]
    InvalidPackageName {
        /// The invalid name.
        name: String,
        /// Why the name is invalid.
        reason: String,
    },

    /// SSRF attempt blocked — tarball URL doesn't match registry.
    #[error("SSRF blocked: tarball URL {url} does not match registry")]
    SsrfBlocked {
        /// The blocked URL.
        url: String,
    },

    /// Security gate denied the operation.
    #[error("security denied: {0}")]
    SecurityDenied(String),

    /// WASM module hash verification failed.
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// Expected blake3 hex digest.
        expected: String,
        /// Actual blake3 hex digest.
        actual: String,
    },

    /// Lockfile read/write/parse error.
    #[error("lockfile error at {path}: {message}")]
    LockfileError {
        /// Path to the lockfile.
        path: PathBuf,
        /// Error description.
        message: String,
    },
}

impl From<astrid_storage::StorageError> for PluginError {
    fn from(e: astrid_storage::StorageError) -> Self {
        Self::Storage(e.to_string())
    }
}

/// Result type for plugin operations.
pub type PluginResult<T> = Result<T, PluginError>;
