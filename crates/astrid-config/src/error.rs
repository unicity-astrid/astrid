use std::io;
use thiserror::Error;

/// Configuration error type.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read configuration file.
    #[error("Failed to read config file at {path}: {source}")]
    ReadError {
        /// Path to the config file that could not be read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// Failed to parse TOML configuration.
    #[error("Failed to parse config file at {path}: {source}")]
    ParseError {
        /// Path to the config file that failed to parse.
        path: String,
        /// Underlying TOML parse error.
        #[source]
        source: toml::de::Error,
    },

    /// Configuration validation failed.
    #[error("Validation error in field '{field}': {message}")]
    ValidationError {
        /// Field that failed validation.
        field: String,
        /// Validation failure description.
        message: String,
    },

    /// Environment variable error.
    #[error("Environment variable '{var_name}': {message}")]
    EnvError {
        /// Name of the environment variable.
        var_name: String,
        /// Error description.
        message: String,
    },

    /// Configuration restriction violated.
    #[error("Restriction violation in field '{field}': {message}")]
    RestrictionViolation {
        /// Field that violates the restriction.
        field: String,
        /// Restriction violation description.
        message: String,
    },

    /// Could not determine home directory.
    #[error("Could not determine home directory")]
    NoHomeDir,
}

/// Result type for configuration operations.
pub type ConfigResult<T> = Result<T, ConfigError>;
