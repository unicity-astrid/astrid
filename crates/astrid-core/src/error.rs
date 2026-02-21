//! Security error types for Astrid operations.

use thiserror::Error;

/// Security-related errors that can occur in Astrid operations.
#[derive(Debug, Error)]
pub enum SecurityError {
    // Crypto errors
    /// Signature verification failed
    #[error("signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    /// Key is not trusted
    #[error("untrusted key: {key_id}")]
    UntrustedKey {
        /// The key identifier that was not trusted
        key_id: String,
    },

    /// Cryptographic operation failed
    #[error("crypto operation failed: {0}")]
    CryptoError(String),

    // Capability errors
    /// Capability token has expired
    #[error("capability token expired: {token_id}")]
    CapabilityExpired {
        /// The expired token identifier
        token_id: String,
    },

    /// Capability token has been revoked
    #[error("capability token revoked: {token_id}")]
    CapabilityRevoked {
        /// The revoked token identifier
        token_id: String,
    },

    /// Insufficient permissions for the requested operation
    #[error("insufficient capability: required {required} for {resource}")]
    InsufficientCapability {
        /// The required permission
        required: String,
        /// The resource being accessed
        resource: String,
    },

    /// Capability token not found
    #[error("capability not found: {token_id}")]
    CapabilityNotFound {
        /// The token identifier that was not found
        token_id: String,
    },

    // Input errors
    /// Untrusted input was rejected
    #[error("untrusted input rejected: {reason}")]
    UntrustedInputRejected {
        /// Reason for rejection
        reason: String,
    },

    /// Invalid input format
    #[error("invalid input: {0}")]
    InvalidInput(String),

    // Approval errors
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

    // Sandbox errors
    /// Sandbox policy violation
    #[error("sandbox violation: {0}")]
    SandboxViolation(String),

    /// Path is outside allowed boundaries
    #[error("path outside sandbox: {path}")]
    PathOutsideSandbox {
        /// The path that was outside the sandbox
        path: String,
    },

    // MCP errors
    /// Not connected to MCP server
    #[error("MCP not connected: {server}")]
    McpNotConnected {
        /// The server that was not connected
        server: String,
    },

    /// MCP tool call was rejected
    #[error("MCP tool rejected: {tool} - {reason}")]
    McpToolRejected {
        /// The tool that was rejected
        tool: String,
        /// Reason for rejection
        reason: String,
    },

    /// MCP elicitation failed
    #[error("MCP elicitation failed: {0}")]
    McpElicitationFailed(String),

    // Audit errors
    /// Failed to write audit entry
    #[error("audit write failed: {0}")]
    AuditWriteFailed(String),

    /// Audit chain integrity violation detected
    #[error("audit integrity violation: {0}")]
    AuditIntegrityViolation(String),

    // Identity errors
    /// Identity not found
    #[error("identity not found: {0}")]
    IdentityNotFound(String),

    /// Identity verification failed
    #[error("identity verification failed: {0}")]
    IdentityVerificationFailed(String),

    /// Frontend link already exists
    #[error("frontend already linked: {frontend} -> {existing_id}")]
    FrontendAlreadyLinked {
        /// The frontend type
        frontend: String,
        /// The existing linked identity
        existing_id: String,
    },

    // Memory access errors
    /// Memory access denied
    #[error("memory access denied: {reason}")]
    MemoryAccessDenied {
        /// Reason for denial
        reason: String,
    },

    /// Memory not found
    #[error("memory not found: {memory_id}")]
    MemoryNotFound {
        /// The memory identifier
        memory_id: String,
    },

    /// Grant creation failed
    #[error("grant creation failed: {0}")]
    GrantCreationFailed(String),

    /// Message verification failed
    #[error("message verification failed: {message_id} - {reason}")]
    MessageVerificationFailed {
        /// The message that failed verification
        message_id: String,
        /// Reason for failure
        reason: String,
    },

    // Verification errors
    /// Verification request expired
    #[error("verification expired")]
    VerificationExpired,

    /// Verification was cancelled
    #[error("verification cancelled")]
    VerificationCancelled,

    // Storage errors
    /// Storage operation failed
    #[error("storage error: {0}")]
    StorageError(String),

    // Generic errors
    /// Internal error
    #[error("internal error: {0}")]
    Internal(String),

    /// Configuration error
    #[error("configuration error: {0}")]
    ConfigurationError(String),
}

/// Result type for security operations.
pub type SecurityResult<T> = Result<T, SecurityError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SecurityError::CapabilityExpired {
            token_id: "abc123".to_string(),
        };
        assert_eq!(err.to_string(), "capability token expired: abc123");

        let err = SecurityError::InsufficientCapability {
            required: "Write".to_string(),
            resource: "file:///etc/passwd".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "insufficient capability: required Write for file:///etc/passwd"
        );
    }

    #[test]
    fn test_result_type() {
        #[allow(clippy::unnecessary_wraps)]
        fn returns_ok() -> SecurityResult<i32> {
            Ok(42)
        }

        fn returns_err() -> SecurityResult<i32> {
            Err(SecurityError::ApprovalDenied {
                reason: "test".to_string(),
            })
        }

        assert!(returns_ok().is_ok());
        assert!(returns_err().is_err());
    }
}
