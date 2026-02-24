use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during capsule operations.
#[derive(Debug, Error)]
pub enum CapsuleError {
    /// Failed to parse the `Capsule.toml` manifest.
    #[error("Failed to parse manifest at {path}: {message}")]
    ManifestParseError {
        /// Path to the invalid manifest.
        path: PathBuf,
        /// The parse error message.
        message: String,
    },
    /// The capsule requests an unsupported entry point or feature.
    #[error("Unsupported entry point: {0}")]
    UnsupportedEntryPoint(String),
}

/// A specialized Result type for capsule operations.
pub type CapsuleResult<T> = Result<T, CapsuleError>;
