use serde::{Deserialize, Serialize};

/// A cryptographic handle representing an open directory within the VFS.
/// This acts as a capability token preventing the guest from forging arbitrary paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DirHandle(pub String);

#[allow(clippy::new_without_default)]
impl DirHandle {
    /// Create a new directory handle.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl std::fmt::Display for DirHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A cryptographic handle representing an open file within the VFS.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileHandle(pub String);

#[allow(clippy::new_without_default)]
impl FileHandle {
    /// Create a new file handle.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl std::fmt::Display for FileHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
