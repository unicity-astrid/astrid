//! Error types for the `OpenClaw` bridge.

use std::path::PathBuf;

/// All errors that can occur during `OpenClaw` → Astralis conversion.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Failed to read or parse the `OpenClaw` manifest.
    #[error("manifest error: {0}")]
    Manifest(String),

    /// The `OpenClaw` plugin ID could not be converted to a valid Astralis `PluginId`.
    #[error("invalid plugin id '{original}': {reason}")]
    InvalidId { original: String, reason: String },

    /// The plugin entry point file was not found.
    #[error("entry point not found: {0}")]
    EntryPointNotFound(PathBuf),

    /// esbuild invocation failed.
    #[error("esbuild failed: {0}")]
    BundleFailed(String),

    /// `extism-js` invocation failed.
    #[error("extism-js compilation failed: {0}")]
    CompileFailed(String),

    /// Failed to write output files.
    #[error("output error: {0}")]
    Output(String),

    /// An external tool is not installed.
    #[error("{tool} not found in PATH. Install: {install_hint}")]
    ToolNotFound { tool: String, install_hint: String },

    /// Generic I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type BridgeResult<T> = Result<T, BridgeError>;
