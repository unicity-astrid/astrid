//! Telemetry error types.

use thiserror::Error;

/// Errors that can occur with telemetry operations.
#[derive(Debug, Error)]
pub enum TelemetryError {
    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Initialization error.
    #[error("Initialization error: {0}")]
    InitError(String),

    /// IO error.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Result type for telemetry operations.
pub type TelemetryResult<T> = Result<T, TelemetryError>;
