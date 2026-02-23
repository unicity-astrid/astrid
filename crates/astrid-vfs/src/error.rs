use thiserror::Error;

/// Virtual filesystem errors.
#[derive(Debug, Error)]
pub enum VfsError {
    /// Sandbox path traversal violation.
    #[error("Path resolves outside sandbox boundaries: {0}")]
    SandboxViolation(String),

    /// Missing handle or capability token.
    #[error("Invalid or unrecognized capability handle")]
    InvalidHandle,

    /// Native IO error wrapper.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Missing directory or file.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Insufficient permission.
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

/// Convenience result type for VFS operations.
pub type VfsResult<T> = Result<T, VfsError>;
