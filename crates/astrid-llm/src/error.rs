//! LLM-related error types.

use thiserror::Error;

/// Errors that can occur with LLM operations.
#[derive(Debug, Error)]
pub enum LlmError {
    /// API key not configured.
    #[error("API key not configured for {provider}")]
    ApiKeyNotConfigured {
        /// Provider name.
        provider: String,
    },

    /// API request failed.
    #[error("API request failed: {0}")]
    ApiRequestFailed(String),

    /// Rate limit exceeded.
    #[error("Rate limit exceeded, retry after {retry_after_secs} seconds")]
    RateLimitExceeded {
        /// Seconds to wait before retrying.
        retry_after_secs: u64,
    },

    /// Invalid response from API.
    #[error("Invalid API response: {0}")]
    InvalidResponse(String),

    /// Model not supported.
    #[error("Model not supported: {model}")]
    ModelNotSupported {
        /// Model name.
        model: String,
    },

    /// Context length exceeded.
    #[error("Context length exceeded: {current} tokens, max is {max}")]
    ContextLengthExceeded {
        /// Current token count.
        current: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// Streaming error.
    #[error("Streaming error: {0}")]
    StreamingError(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// HTTP error.
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Result type for LLM operations.
pub type LlmResult<T> = Result<T, LlmError>;
