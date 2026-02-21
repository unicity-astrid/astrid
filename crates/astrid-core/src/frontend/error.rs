/// Errors that can occur during frontend operations.
#[derive(Debug, thiserror::Error)]
pub enum FrontendError {
    /// Approval was denied by the user
    #[error("approval denied: {reason}")]
    ApprovalDenied {
        /// Reason for denial
        reason: String,
    },

    /// Approval request timed out
    #[error("approval timeout after {timeout_ms}ms")]
    ApprovalTimeout {
        /// Timeout duration in milliseconds
        timeout_ms: u64,
    },

    /// MCP elicitation failed
    #[error("elicitation failed: {0}")]
    ElicitationFailed(String),

    /// Internal frontend error
    #[error("internal frontend error: {0}")]
    Internal(String),

    /// Underlying security error
    #[error(transparent)]
    Security(#[from] crate::error::SecurityError),
}

/// Result type for frontend operations.
pub type FrontendResult<T> = Result<T, FrontendError>;
