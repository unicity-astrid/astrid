/// Errors that can occur during approval and security interception.
#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    /// Action requires approval but none was granted.
    #[error("approval denied: {reason}")]
    Denied {
        /// The reason the action was denied.
        reason: String,
    },

    /// The approval request timed out.
    #[error("approval timeout after {timeout_ms}ms")]
    Timeout {
        /// Time awaited before timeout, in milliseconds.
        timeout_ms: u64,
    },

    /// The action is blocked by security policy.
    #[error("blocked by policy: {tool} - {reason}")]
    PolicyBlocked {
        /// The tool or action being blocked.
        tool: String,
        /// The reason for blocking.
        reason: String,
    },

    /// Storage backend error (lock poisoned, persistence failed, etc.).
    #[error("storage error: {0}")]
    Storage(String),

    /// Internal approval system error.
    #[error("internal approval error: {0}")]
    Internal(String),
}

/// Result type for approval operations.
pub type ApprovalResult<T> = Result<T, ApprovalError>;
