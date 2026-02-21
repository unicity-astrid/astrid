use crate::error::SecurityError;

// ConnectorError
// ---------------------------------------------------------------------------

/// Errors specific to connector operations.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    /// The connector is not connected or has been unregistered.
    #[error("connector not connected")]
    NotConnected,

    /// Sending a message failed.
    #[error("send failed: {0}")]
    SendFailed(String),

    /// The plugin ID failed validation (must be non-empty, lowercase
    /// alphanumeric and hyphens, must not start or end with a hyphen).
    #[error("invalid plugin id: {0}")]
    InvalidPluginId(String),

    /// The requested operation is not supported by this connector.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// Serialization / deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// An underlying security error.
    #[error(transparent)]
    Security(#[from] SecurityError),

    /// Catch-all for internal errors.
    #[error("internal connector error: {0}")]
    Internal(String),
}

/// Convenience alias for connector operations.
pub type ConnectorResult<T> = Result<T, ConnectorError>;

// ---------------------------------------------------------------------------
