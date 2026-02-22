use astrid_capabilities::{DirHandle, FileHandle};
use async_trait::async_trait;

use crate::{Vfs, VfsDirEntry, VfsMetadata, VfsResult};

use std::sync::Arc;
use tokio::sync::Mutex;
use dashmap::DashMap;

/// An implementation of `Vfs` providing a Copy-on-Write overlay.
/// Reads fall through to the lower filesystem if absent in the upper.
/// Writes strictly apply only to the upper filesystem.
pub struct OverlayVfs {
    lower: Box<dyn Vfs>,
    upper: Box<dyn Vfs>,
    // Synchronize concurrent copy-ups per path
    copy_locks: DashMap<String, Arc<Mutex<()>>>,
}

impl OverlayVfs {
    /// Create a new Overlay VFS.
    #[must_use]
    pub fn new(lower: Box<dyn Vfs>, upper: Box<dyn Vfs>) -> Self {
        Self { lower, upper, copy_locks: DashMap::new() }
    }
}

#[async_trait]
impl Vfs for OverlayVfs {
    async fn exists(&self, handle: &DirHandle, path: &str) -> VfsResult<bool> {
        if self.upper.exists(handle, path).await? {
            return Ok(true);
        }
        self.lower.exists(handle, path).await
    }

    async fn readdir(&self, handle: &DirHandle, path: &str) -> VfsResult<Vec<VfsDirEntry>> {
        // Simple merge: read from both and deduplicate by name, preferring upper.
        let mut entries = std::collections::HashMap::new();
        let mut lower_success = false;

        if let Ok(lower_entries) = self.lower.readdir(handle, path).await {
            lower_success = true;
            for entry in lower_entries {
                entries.insert(entry.name.clone(), entry);
            }
        }

        match self.upper.readdir(handle, path).await {
            Ok(upper_entries) => {
                for entry in upper_entries {
                    entries.insert(entry.name.clone(), entry);
                }
            }
            Err(crate::VfsError::NotFound(_) | crate::VfsError::InvalidHandle) => {
                // If upper just couldn't find the dir, it's fine as long as lower did
                if !lower_success {
                    return Err(crate::VfsError::NotFound(path.into()));
                }
            }
            Err(e) => return Err(e), // Propagate hard IO/Permission errors immediately
        }

        Ok(entries.into_values().collect())
    }

    async fn stat(&self, handle: &DirHandle, path: &str) -> VfsResult<VfsMetadata> {
        if let Ok(meta) = self.upper.stat(handle, path).await {
            return Ok(meta);
        }
        self.lower.stat(handle, path).await
    }

    async fn mkdir(&self, handle: &DirHandle, path: &str) -> VfsResult<()> {
        // Writes strictly to upper.
        self.upper.mkdir(handle, path).await
    }

    async fn unlink(&self, handle: &DirHandle, path: &str) -> VfsResult<()> {
        // Writes strictly to upper.
        // Note: fully correct overlayfs requires recording a "whiteout" or tombstone
        // in the upper layer to hide a lower layer file. We omit whiteouts here for simplicity.
        if self.lower.exists(handle, path).await.unwrap_or(false) {
            return Err(crate::VfsError::PermissionDenied(
                "Cannot delete read-only workspace file (whiteout support not implemented)".into(),
            ));
        }
        self.upper.unlink(handle, path).await
    }

    async fn open(&self, handle: &DirHandle, path: &str, write: bool, truncate: bool) -> VfsResult<FileHandle> {
        if write {
            // Write operations strictly against upper.
            // Copy-up logic would occur here if modifying an existing lower file.
            // For now, assume creation in upper.
            let needs_copy = !self.upper.exists(handle, path).await.unwrap_or(false)
                && self.lower.exists(handle, path).await.unwrap_or(false);

            if needs_copy {
                // Ensure only one task performs the copy-up for this path
                let normalized_path = crate::path::resolve_path(std::path::Path::new("/"), path)
                    .map_or_else(|_| path.to_string(), |p| p.to_string_lossy().to_string());
                let lock_key = normalized_path;
                
                let path_lock = self.copy_locks.entry(lock_key).or_insert_with(|| Arc::new(Mutex::new(()))).clone();
                let _guard = path_lock.lock().await;

                // Re-check after acquiring the lock in case another task already copied it
                if !self.upper.exists(handle, path).await.unwrap_or(false) {
                    if truncate {
                        // Fast path: if truncating, just create an empty file in upper
                        // We don't need to copy the contents from lower
                        let new_upper_file = self.upper.open(handle, path, true, true).await?;
                        let _ = self.upper.close(&new_upper_file).await;
                    } else {
                        // Prevent OOM during copy-up by capping the size
                        let meta = self.lower.stat(handle, path).await?;
                        if meta.size > 50 * 1024 * 1024 {
                            return Err(crate::VfsError::PermissionDenied(
                                "File is too large for OverlayVfs copy-up (> 50MB)".into(),
                            ));
                        }

                        // Perform copy-up.
                        let lower_handle = self.lower.open(handle, path, false, false).await?;
                        let content_result = self.lower.read(&lower_handle).await;

                        // Ensure lower is closed even if read fails
                        let _ = self.lower.close(&lower_handle).await;

                        let content = content_result?;

                        let new_upper_file = self.upper.open(handle, path, true, true).await?;
                        let write_result = self.upper.write(&new_upper_file, &content).await;

                        // Ensure upper is closed even if write fails
                        let _ = self.upper.close(&new_upper_file).await;

                        if let Err(e) = write_result {
                            // Revert the copy-up so we don't leave a truncated file
                            let _ = self.upper.unlink(handle, path).await;
                            return Err(e);
                        }
                    }
                }
            }
            return self.upper.open(handle, path, write, truncate).await;
        }

        // Read-only logic.
        if self.upper.exists(handle, path).await.unwrap_or(false) {
            return self.upper.open(handle, path, false, false).await;
        }

        self.lower.open(handle, path, false, false).await
    }

    async fn open_dir(&self, handle: &DirHandle, path: &str, new_handle: DirHandle) -> VfsResult<()> {
        let exists_upper = self.upper.exists(handle, path).await.unwrap_or(false);
        let lower_meta = self.lower.stat(handle, path).await;

        if exists_upper {
            self.upper.open_dir(handle, path, new_handle.clone()).await?;
            if matches!(lower_meta, Ok(meta) if meta.is_dir) {
                // Propagate failures to ensure lower capabilities are mapped, or correctly fail
                if let Err(e) = self.lower.open_dir(handle, path, new_handle.clone()).await {
                    let _ = self.upper.close_dir(&new_handle).await;
                    return Err(e);
                }
            }
        } else if let Ok(meta) = lower_meta {
            if meta.is_dir {
                // Eagerly create the directory in upper to ensure symmetric handle mapping
                self.upper.mkdir(handle, path).await.unwrap_or(());
                self.upper.open_dir(handle, path, new_handle.clone()).await?;
                if let Err(e) = self.lower.open_dir(handle, path, new_handle.clone()).await {
                    let _ = self.upper.close_dir(&new_handle).await;
                    return Err(e);
                }
            } else {
                return Err(crate::VfsError::PermissionDenied(
                    "Cannot open a file as a directory".into(),
                ));
            }
        } else {
            return Err(crate::VfsError::NotFound(path.into()));
        }

        Ok(())
    }

    async fn close_dir(&self, handle: &DirHandle) -> VfsResult<()> {
        let _ = self.upper.close_dir(handle).await;
        let _ = self.lower.close_dir(handle).await;
        Ok(())
    }

    async fn read(&self, handle: &FileHandle) -> VfsResult<Vec<u8>> {
        // We don't know which underlying system owns the FileHandle. Try upper, then lower.
        match self.upper.read(handle).await {
            Ok(data) => Ok(data),
            Err(crate::VfsError::InvalidHandle) => self.lower.read(handle).await,
            Err(e) => Err(e),
        }
    }

    async fn write(&self, handle: &FileHandle, content: &[u8]) -> VfsResult<()> {
        // Writes always go to upper. If it's a lower handle, this is an error
        // since lower files are opened read-only.
        self.upper.write(handle, content).await
    }

    async fn close(&self, handle: &FileHandle) -> VfsResult<()> {
        let upper_res = self.upper.close(handle).await;
        let lower_res = self.lower.close(handle).await;

        if upper_res.is_ok() || lower_res.is_ok() {
            Ok(())
        } else {
            Err(crate::VfsError::InvalidHandle)
        }
    }
}
