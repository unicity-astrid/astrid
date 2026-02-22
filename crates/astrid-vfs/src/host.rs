use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use astrid_capabilities::{DirHandle, FileHandle};
use async_trait::async_trait;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;

use crate::path::resolve_path;
use crate::{Vfs, VfsDirEntry, VfsError, VfsMetadata, VfsResult};

/// An implementation of `Vfs` backed by the physical host filesystem.
pub struct HostVfs {
    open_dirs: RwLock<HashMap<DirHandle, PathBuf>>,
    open_files: RwLock<HashMap<FileHandle, Arc<RwLock<fs::File>>>>,
}

impl HostVfs {
    /// Create a new host VFS.
    #[must_use]
    pub fn new() -> Self {
        Self {
            open_dirs: RwLock::new(HashMap::new()),
            open_files: RwLock::new(HashMap::new()),
        }
    }

    /// Register a root directory capability manually (e.g. from the Daemon).
    pub async fn register_dir(&self, handle: DirHandle, physical_path: PathBuf) {
        let mut dirs = self.open_dirs.write().await;
        dirs.insert(handle, physical_path);
    }

    async fn get_dir_path(&self, handle: &DirHandle) -> VfsResult<PathBuf> {
        let dirs = self.open_dirs.read().await;
        dirs.get(handle).cloned().ok_or(VfsError::InvalidHandle)
    }

    async fn resolve_physical_path(&self, handle: &DirHandle, path: &str) -> VfsResult<PathBuf> {
        let base = self.get_dir_path(handle).await?;
        let resolved = resolve_path(&base, path)?;

        // Protect against symlink traversal sandbox escapes
        if tokio::fs::try_exists(&resolved).await.unwrap_or(false) {
            let canonical = tokio::fs::canonicalize(&resolved).await.map_err(VfsError::from)?;
            let canonical_base = tokio::fs::canonicalize(&base).await.unwrap_or_else(|_| base.clone());
            if !canonical.starts_with(&canonical_base) {
                return Err(VfsError::SandboxViolation(
                    "Path resolves outside sandbox boundaries via symlink".into(),
                ));
            }
        } else if let Some(parent) = resolved.parent() {
            // If creating a new file, check the parent directory
            if tokio::fs::try_exists(parent).await.unwrap_or(false) {
                let canonical_parent = tokio::fs::canonicalize(parent).await.map_err(VfsError::from)?;
                let canonical_base = tokio::fs::canonicalize(&base).await.unwrap_or_else(|_| base.clone());
                if !canonical_parent.starts_with(&canonical_base) {
                    return Err(VfsError::SandboxViolation(
                        "Parent path resolves outside sandbox boundaries via symlink".into(),
                    ));
                }
            }
        }

        Ok(resolved)
    }
}

impl Default for HostVfs {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Vfs for HostVfs {
    async fn exists(&self, handle: &DirHandle, path: &str) -> VfsResult<bool> {
        let target = self.resolve_physical_path(handle, path).await?;
        Ok(tokio::fs::try_exists(&target).await.unwrap_or(false))
    }

    async fn readdir(&self, handle: &DirHandle, path: &str) -> VfsResult<Vec<VfsDirEntry>> {
        let target = self.resolve_physical_path(handle, path).await?;
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&target).await.map_err(VfsError::from)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(VfsError::from)? {
            let metadata = entry.metadata().await.map_err(VfsError::from)?;
            entries.push(VfsDirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: metadata.is_dir(),
            });
        }
        Ok(entries)
    }

    async fn stat(&self, handle: &DirHandle, path: &str) -> VfsResult<VfsMetadata> {
        let target = self.resolve_physical_path(handle, path).await?;
        let metadata = tokio::fs::metadata(&target).await.map_err(VfsError::from)?;
        Ok(VfsMetadata {
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            size: metadata.len(),
        })
    }

    async fn mkdir(&self, handle: &DirHandle, path: &str) -> VfsResult<()> {
        let target = self.resolve_physical_path(handle, path).await?;
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(VfsError::from)
    }

    async fn unlink(&self, handle: &DirHandle, path: &str) -> VfsResult<()> {
        let target = self.resolve_physical_path(handle, path).await?;
        let meta = tokio::fs::symlink_metadata(&target)
            .await
            .map_err(VfsError::from)?;
        if meta.is_dir() {
            tokio::fs::remove_dir(&target)
                .await
                .map_err(VfsError::from)
        } else {
            tokio::fs::remove_file(&target)
                .await
                .map_err(VfsError::from)
        }
    }

    async fn open(&self, handle: &DirHandle, path: &str, write: bool) -> VfsResult<FileHandle> {
        let target = self.resolve_physical_path(handle, path).await?;

        let mut files = self.open_files.write().await;
        if files.len() >= 64 {
            return Err(VfsError::PermissionDenied("Too many open files".into()));
        }

        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .create(write)
            .open(&target)
            .await
            .map_err(VfsError::from)?;

        let new_handle = FileHandle::new();
        files.insert(new_handle.clone(), Arc::new(RwLock::new(file)));

        Ok(new_handle)
    }

    async fn open_dir(&self, handle: &DirHandle, path: &str) -> VfsResult<DirHandle> {
        let target = self.resolve_physical_path(handle, path).await?;

        // Ensure it's a directory
        let meta = tokio::fs::metadata(&target).await.map_err(VfsError::from)?;
        if !meta.is_dir() {
            return Err(VfsError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "Target is not a directory",
            )));
        }

        let mut dirs = self.open_dirs.write().await;
        if dirs.len() >= 64 {
            return Err(VfsError::PermissionDenied("Too many open directories".into()));
        }
        
        let new_handle = DirHandle::new();
        dirs.insert(new_handle.clone(), target);
        Ok(new_handle)
    }

    async fn close_dir(&self, handle: &DirHandle) -> VfsResult<()> {
        let mut dirs = self.open_dirs.write().await;
        if dirs.remove(handle).is_none() {
            return Err(VfsError::InvalidHandle);
        }
        Ok(())
    }

    async fn read(&self, handle: &FileHandle) -> VfsResult<Vec<u8>> {
        let file_arc = {
            let files = self.open_files.read().await;
            files.get(handle).cloned().ok_or(VfsError::InvalidHandle)?
        };

        let mut file = file_arc.write().await;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .await
            .map_err(VfsError::from)?;
        Ok(buffer)
    }

    async fn write(&self, handle: &FileHandle, content: &[u8]) -> VfsResult<()> {
        let file_arc = {
            let files = self.open_files.read().await;
            files.get(handle).cloned().ok_or(VfsError::InvalidHandle)?
        };

        let mut file = file_arc.write().await;
        file.write_all(content).await.map_err(VfsError::from)?;
        file.flush().await.map_err(VfsError::from)?;
        Ok(())
    }

    async fn close(&self, handle: &FileHandle) -> VfsResult<()> {
        let mut files = self.open_files.write().await;
        if files.remove(handle).is_none() {
            return Err(VfsError::InvalidHandle);
        }
        Ok(())
    }
}
