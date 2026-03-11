/// Errors that can occur during identity operations.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// Identity not found
    #[error("identity not found: {0}")]
    NotFound(String),

    /// Identity verification failed
    #[error("identity verification failed: {0}")]
    VerificationFailed(String),

    /// Platform link already exists
    #[error("platform already linked: {platform} -> {existing_id}")]
    PlatformAlreadyLinked {
        /// The platform name
        platform: String,
        /// The existing linked identity
        existing_id: String,
    },

    /// Verification request expired
    #[error("verification expired")]
    VerificationExpired,

    /// Internal identity error
    #[error("internal identity error: {0}")]
    Internal(String),
}

/// Result type for identity operations.
pub type IdentityResult<T> = Result<T, IdentityError>;
