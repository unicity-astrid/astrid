//! MCP-related error types.

use thiserror::Error;

/// Errors that can occur with MCP operations.
#[derive(Debug, Error)]
pub enum McpError {
    /// Server not found.
    #[error("MCP server not found: {name}")]
    ServerNotFound {
        /// The server name that was not found.
        name: String,
    },

    /// Server already running.
    #[error("MCP server already running: {name}")]
    ServerAlreadyRunning {
        /// The server name.
        name: String,
    },

    /// Server not running.
    #[error("MCP server not running: {name}")]
    ServerNotRunning {
        /// The server name.
        name: String,
    },

    /// Failed to start server.
    #[error("Failed to start MCP server {name}: {reason}")]
    ServerStartFailed {
        /// The server name.
        name: String,
        /// Reason for failure.
        reason: String,
    },

    /// Connection failed.
    #[error("MCP connection failed: {0}")]
    ConnectionFailed(String),

    /// Tool not found.
    #[error("Tool not found: {server}:{tool}")]
    ToolNotFound {
        /// Server name.
        server: String,
        /// Tool name.
        tool: String,
    },

    /// Tool call failed.
    #[error("Tool call failed: {server}:{tool} - {reason}")]
    ToolCallFailed {
        /// Server name.
        server: String,
        /// Tool name.
        tool: String,
        /// Reason for failure.
        reason: String,
    },

    /// Authorization required.
    #[error("Authorization required for {server}:{tool}")]
    AuthorizationRequired {
        /// Server name.
        server: String,
        /// Tool name.
        tool: String,
    },

    /// Binary hash mismatch.
    #[error("Binary hash mismatch for {name}: expected {expected}, got {actual}")]
    BinaryHashMismatch {
        /// Server name.
        name: String,
        /// Expected hash.
        expected: String,
        /// Actual hash.
        actual: String,
    },

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// IO error.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Transport error.
    #[error("Transport error: {0}")]
    TransportError(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Timeout.
    #[error("Operation timed out")]
    Timeout,

    /// MCP protocol error from rmcp.
    #[error("MCP protocol error: {0}")]
    ProtocolError(String),

    /// MCP initialization failed.
    #[error("MCP initialization failed: {0}")]
    InitializationFailed(String),
}

impl From<rmcp::ServiceError> for McpError {
    fn from(err: rmcp::ServiceError) -> Self {
        Self::ProtocolError(err.to_string())
    }
}

impl From<rmcp::service::ClientInitializeError> for McpError {
    fn from(err: rmcp::service::ClientInitializeError) -> Self {
        Self::InitializationFailed(err.to_string())
    }
}

/// Result type for MCP operations.
pub type McpResult<T> = Result<T, McpError>;
