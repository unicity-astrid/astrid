//! Plugin error types.

use std::path::PathBuf;

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
}

impl From<astralis_storage::StorageError> for PluginError {
    fn from(e: astralis_storage::StorageError) -> Self {
        Self::Storage(e.to_string())
    }
}

/// Result type for plugin operations.
pub type PluginResult<T> = Result<T, PluginError>;
