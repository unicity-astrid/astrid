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

    /// The capsule ID failed validation (must be non-empty, lowercase
    /// alphanumeric and hyphens, must not start or end with a hyphen).
    #[error("invalid capsule id: {0}")]
    InvalidCapsuleId(String),

    /// The requested operation is not supported by this uplink.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),
}

/// Convenience alias for uplink operations.
pub type UplinkResult<T> = Result<T, UplinkError>;

// ---------------------------------------------------------------------------
