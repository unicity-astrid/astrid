//! Runtime error types.

use thiserror::Error;

/// Errors that can occur in the runtime.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Session not found.
    #[error("Session not found: {session_id}")]
    SessionNotFound {
        /// The session ID.
        session_id: String,
    },

    /// Session already exists.
    #[error("Session already exists: {session_id}")]
    SessionExists {
        /// The session ID.
        session_id: String,
    },

    /// LLM error.
    #[error("LLM error: {0}")]
    LlmError(#[from] astrid_llm::LlmError),

    /// MCP error.
    #[error("MCP error: {0}")]
    McpError(#[from] astrid_mcp::McpError),

    /// Audit error.
    #[error("Audit error: {0}")]
    AuditError(#[from] astrid_audit::AuditError),

    /// Capability error.
    #[error("Capability error: {0}")]
    CapabilityError(#[from] astrid_capabilities::CapabilityError),

    /// Security error.
    #[error("Security error: {0}")]
    SecurityError(#[from] astrid_core::SecurityError),

    /// Storage error.
    #[error("Storage error: {0}")]
    StorageError(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Context overflow.
    #[error("Context overflow: {current} tokens exceeds limit of {max}")]
    ContextOverflow {
        /// Current token count.
        current: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// Approval required.
    #[error("Approval required for: {action}")]
    ApprovalRequired {
        /// The action requiring approval.
        action: String,
    },

    /// Approval denied.
    #[error("Approval denied: {reason}")]
    ApprovalDenied {
        /// Reason for denial.
        reason: String,
    },

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Sub-agent error.
    #[error("Sub-agent error: {0}")]
    SubAgentError(String),

    /// IO error.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Result type for runtime operations.
pub type RuntimeResult<T> = Result<T, RuntimeError>;
