//! Cryptographic error types.

use thiserror::Error;

/// Errors that can occur during cryptographic operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Invalid key length.
    #[error("invalid key length: expected {expected}, got {actual}")]
    InvalidKeyLength {
        /// Expected length in bytes.
        expected: usize,
        /// Actual length in bytes.
        actual: usize,
    },

    /// Invalid signature length.
    #[error("invalid signature length: expected {expected}, got {actual}")]
    InvalidSignatureLength {
        /// Expected length in bytes.
        expected: usize,
        /// Actual length in bytes.
        actual: usize,
    },

    /// Invalid public key.
    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),

    /// Signature verification failed.
    #[error("signature verification failed")]
    SignatureVerificationFailed,

    /// Invalid hex encoding.
    #[error("invalid hex encoding")]
    InvalidHexEncoding,

    /// Invalid base64 encoding.
    #[error("invalid base64 encoding")]
    InvalidBase64Encoding,

    /// I/O error (e.g. reading/writing key files).
    #[error("I/O error: {0}")]
    IoError(String),
}

/// Result type for cryptographic operations.
pub type CryptoResult<T> = Result<T, CryptoError>;
