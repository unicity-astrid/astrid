// UplinkError
// ---------------------------------------------------------------------------

/// Errors specific to uplink operations.
#[derive(Debug, thiserror::Error)]
pub enum UplinkError {
    /// The uplink is not connected or has been unregistered.
    #[error("uplink not connected")]
    NotConnected,

    /// Sending a message failed.
    #[error("send failed: {0}")]
    SendFailed(String),

    /// The plugin ID failed validation (must be non-empty, lowercase
    /// alphanumeric and hyphens, must not start or end with a hyphen).
    #[error("invalid plugin id: {0}")]
    InvalidPluginId(String),

    /// The requested operation is not supported by this uplink.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// Serialization / deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// An approval request was denied by the user.
    #[error("approval denied: {reason}")]
    ApprovalDenied {
        /// The reason for denial.
        reason: String,
    },

    /// An approval request timed out before the user responded.
    #[error("approval timeout after {timeout_ms}ms")]
    ApprovalTimeout {
        /// Timeout duration in milliseconds.
        timeout_ms: u64,
    },

    /// Catch-all for internal errors.
    #[error("internal uplink error: {0}")]
    Internal(String),
}

/// Convenience alias for uplink operations.
pub type UplinkResult<T> = Result<T, UplinkError>;

// ---------------------------------------------------------------------------
