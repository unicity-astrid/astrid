//! Error types for the gateway.

use thiserror::Error;

/// Gateway error type.
#[derive(Debug, Error)]
pub enum GatewayError {
    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Secret loading error.
    #[error("secret error: {0}")]
    Secret(String),

    /// Agent error.
    #[error("agent error: {0}")]
    Agent(String),

    /// Routing error.
    #[error("routing error: {0}")]
    Routing(String),

    /// State persistence error.
    #[error("state error: {0}")]
    State(String),

    /// Health check error.
    #[error("health check error: {0}")]
    Health(String),

    /// Runtime error.
    #[error("runtime error: {0}")]
    Runtime(String),

    /// Shutdown error.
    #[error("shutdown error: {0}")]
    Shutdown(String),

    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// TOML parsing error.
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),

    /// JSON error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Core error.
    #[error("core error: {0}")]
    Core(#[from] astralis_core::SecurityError),

    /// Audit error.
    #[error("audit error: {0}")]
    Audit(#[from] astralis_audit::AuditError),
}

/// Result type for gateway operations.
pub type GatewayResult<T> = Result<T, GatewayError>;
