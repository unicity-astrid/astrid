/// Errors that can occur during identity operations.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// Identity not found
    #[error("identity not found: {0}")]
    NotFound(String),

    /// Identity verification failed
    #[error("identity verification failed: {0}")]
    VerificationFailed(String),

    /// Frontend link already exists
    #[error("frontend already linked: {frontend} -> {existing_id}")]
    FrontendAlreadyLinked {
        /// The frontend type
        frontend: String,
        /// The existing linked identity
        existing_id: String,
    },

    /// Verification request expired
    #[error("verification expired")]
    VerificationExpired,

    /// Verification was cancelled
    #[error("verification cancelled")]
    VerificationCancelled,

    /// Internal identity error
    #[error("internal identity error: {0}")]
    Internal(String),
}

/// Result type for identity operations.
pub type IdentityResult<T> = Result<T, IdentityError>;
