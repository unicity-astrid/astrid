//! Astrid Virtual File System (VFS).
//!
//! Provides an abstraction over filesystem operations to support strict sandboxing,
//! capability-based access, and overlay (copy-on-write) implementations.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

/// Security boundary enforcement via ignore rules.
pub mod boundary;
/// Virtual filesystem error types.
pub mod error;
/// Host-backed virtual filesystem implementation.
pub mod host;
/// Overlay (copy-on-write) virtual filesystem implementation.
pub mod overlay;
/// Path resolution and sandboxing utilities.
pub mod path;
/// Worktree-specific virtual filesystem implementation.
pub mod worktree;

pub use boundary::IgnoreBoundary;
pub use error::{VfsError, VfsResult};
pub use host::HostVfs;
pub use overlay::OverlayVfs;
pub use worktree::WorktreeVfs;

use astrid_capabilities::{DirHandle, FileHandle};
use async_trait::async_trait;

/// File metadata returned by stat.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VfsMetadata {
    /// True if the entry is a directory.
    pub is_dir: bool,
    /// True if the entry is a file.
    pub is_file: bool,
    /// Size of the file in bytes.
    pub size: u64,
    /// Modification time in seconds since the UNIX epoch.
    pub mtime: u64,
}

/// Directory entry returned by readdir.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VfsDirEntry {
    /// Name of the entry.
    pub name: String,
    /// True if the entry is a directory.
    pub is_dir: bool,
}

/// A core virtual filesystem providing sandboxed operations.
#[async_trait]
pub trait Vfs: Send + Sync {
    /// Check if a path exists within the sandbox.
    async fn exists(&self, handle: &DirHandle, path: &str) -> VfsResult<bool>;

    /// Read the contents of a directory.
    async fn readdir(&self, handle: &DirHandle, path: &str) -> VfsResult<Vec<VfsDirEntry>>;

    /// Get metadata for a path.
    async fn stat(&self, handle: &DirHandle, path: &str) -> VfsResult<VfsMetadata>;

    /// Create a new directory.
    async fn mkdir(&self, handle: &DirHandle, path: &str) -> VfsResult<()>;

    /// Remove a file.
    async fn unlink(&self, handle: &DirHandle, path: &str) -> VfsResult<()>;

    /// Open a file for reading/writing. Returns a handle.
    async fn open(
        &self,
        handle: &DirHandle,
        path: &str,
        write: bool,
        truncate: bool,
    ) -> VfsResult<FileHandle>;

    /// Open a subdirectory, granting a new narrowed capability handle.
    async fn open_dir(
        &self,
        handle: &DirHandle,
        path: &str,
        new_handle: DirHandle,
    ) -> VfsResult<()>;

    /// Close a directory handle.
    async fn close_dir(&self, handle: &DirHandle) -> VfsResult<()>;

    /// Read from an open file handle.
    async fn read(&self, handle: &FileHandle) -> VfsResult<Vec<u8>>;

    /// Write to an open file handle.
    async fn write(&self, handle: &FileHandle, content: &[u8]) -> VfsResult<()>;

    /// Close a file handle.
    async fn close(&self, handle: &FileHandle) -> VfsResult<()>;
}
