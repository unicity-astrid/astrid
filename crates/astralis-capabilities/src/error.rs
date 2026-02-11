//! Capability-related error types.

use thiserror::Error;

/// Errors that can occur with capability tokens.
#[derive(Debug, Error)]
pub enum CapabilityError {
    /// Token has expired.
    #[error("capability token expired: {token_id}")]
    TokenExpired {
        /// The expired token ID.
        token_id: String,
    },

    /// Token has been revoked.
    #[error("capability token revoked: {token_id}")]
    TokenRevoked {
        /// The revoked token ID.
        token_id: String,
    },

    /// Token not found.
    #[error("capability token not found: {token_id}")]
    TokenNotFound {
        /// The token ID that was not found.
        token_id: String,
    },

    /// Single-use token has already been used (replay attempt).
    #[error("single-use token already used: {token_id}")]
    TokenAlreadyUsed {
        /// The token ID that was already used.
        token_id: String,
    },

    /// Insufficient permissions.
    #[error("insufficient capability: required {required} for {resource}")]
    InsufficientPermission {
        /// The required permission.
        required: String,
        /// The resource being accessed.
        resource: String,
    },

    /// Invalid token signature.
    #[error("invalid token signature")]
    InvalidSignature,

    /// Invalid resource pattern.
    #[error("invalid resource pattern: {pattern} - {reason}")]
    InvalidPattern {
        /// The invalid pattern.
        pattern: String,
        /// Why it's invalid.
        reason: String,
    },

    /// Storage error.
    #[error("storage error: {0}")]
    StorageError(String),

    /// Crypto error.
    #[error("crypto error: {0}")]
    CryptoError(#[from] astralis_crypto::CryptoError),

    /// Serialization error.
    #[error("serialization error: {0}")]
    SerializationError(String),
}

/// Result type for capability operations.
pub type CapabilityResult<T> = Result<T, CapabilityError>;
