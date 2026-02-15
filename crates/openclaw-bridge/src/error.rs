//! Error types for the `OpenClaw` bridge.

use std::path::PathBuf;

/// All errors that can occur during `OpenClaw` → Astrid conversion.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Failed to read or parse the `OpenClaw` manifest.
    #[error("manifest error: {0}")]
    Manifest(String),

    /// The `OpenClaw` plugin ID could not be converted to a valid Astrid `PluginId`.
    #[error("invalid plugin id '{original}': {reason}")]
    InvalidId { original: String, reason: String },

    /// The plugin entry point file was not found.
    #[error("entry point not found: {0}")]
    EntryPointNotFound(PathBuf),

    /// OXC parse or transform failed.
    #[error("transpile failed: {0}")]
    TranspileFailed(String),

    /// Plugin source contains unresolved import statements.
    #[error("unresolved imports: {0}")]
    UnresolvedImports(String),

    /// JS → WASM compilation failed (Wizer / kernel).
    #[error("compilation failed: {0}")]
    CompileFailed(String),

    /// WASM export stitching failed (wasmparser / wasm-encoder).
    #[error("export stitch failed: {0}")]
    ExportStitchFailed(String),

    /// Compilation cache error (read/write/gc).
    #[error("cache error: {0}")]
    Cache(String),

    /// Failed to write output files.
    #[error("output error: {0}")]
    Output(String),

    /// Generic I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type BridgeResult<T> = Result<T, BridgeError>;
