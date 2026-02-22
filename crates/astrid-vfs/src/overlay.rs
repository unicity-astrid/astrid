use astrid_capabilities::{DirHandle, FileHandle};
use async_trait::async_trait;

use crate::{Vfs, VfsDirEntry, VfsMetadata, VfsResult};

/// An implementation of `Vfs` providing a Copy-on-Write overlay.
/// Reads fall through to the lower filesystem if absent in the upper.
/// Writes strictly apply only to the upper filesystem.
pub struct OverlayVfs {
    lower: Box<dyn Vfs>,
    upper: Box<dyn Vfs>,
    // Map overlay handle to lower and upper handles.
    // In a fully developed implementation, we would maintain state mapping
    // an Overlay DirHandle to its corresponding Lower and Upper DirHandles.
    // For simplicity in this base abstraction, we assume the same handle
    // is registered in both underlying Vfs instances.
}

impl OverlayVfs {
    /// Create a new Overlay VFS.
    #[must_use]
    pub fn new(lower: Box<dyn Vfs>, upper: Box<dyn Vfs>) -> Self {
        Self { lower, upper }
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

        if let Ok(lower_entries) = self.lower.readdir(handle, path).await {
            for entry in lower_entries {
                entries.insert(entry.name.clone(), entry);
            }
        }

        if let Ok(upper_entries) = self.upper.readdir(handle, path).await {
            for entry in upper_entries {
                entries.insert(entry.name.clone(), entry);
            }
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
        self.upper.unlink(handle, path).await
    }

    async fn open(&self, handle: &DirHandle, path: &str, write: bool) -> VfsResult<FileHandle> {
        if write {
            // Write operations strictly against upper.
            // Copy-up logic would occur here if modifying an existing lower file.
            // For now, assume creation in upper.
            if !self.upper.exists(handle, path).await.unwrap_or(false)
                && self.lower.exists(handle, path).await.unwrap_or(false)
            {
                // Prevent OOM during copy-up by capping the size
                let meta = self.lower.stat(handle, path).await?;
                if meta.size > 50 * 1024 * 1024 {
                    return Err(crate::VfsError::PermissionDenied(
                        "File is too large for OverlayVfs copy-up (> 50MB)".into(),
                    ));
                }

                // Perform copy-up.
                let lower_handle = self.lower.open(handle, path, false).await?;
                let content_result = self.lower.read(&lower_handle).await;

                // Ensure lower is closed even if read fails
                let _ = self.lower.close(&lower_handle).await;

                let content = content_result?;

                let new_upper_file = self.upper.open(handle, path, true).await?;
                let write_result = self.upper.write(&new_upper_file, &content).await;

                // Ensure upper is closed even if write fails
                let _ = self.upper.close(&new_upper_file).await;

                write_result?;
            }
            return self.upper.open(handle, path, write).await;
        }

        // Read-only logic.
        if self.upper.exists(handle, path).await.unwrap_or(false) {
            return self.upper.open(handle, path, false).await;
        }

        self.lower.open(handle, path, false).await
    }

    async fn open_dir(&self, handle: &DirHandle, path: &str) -> VfsResult<DirHandle> {
        // Assuming symmetric handle registration for this abstraction base.
        if self.upper.exists(handle, path).await.unwrap_or(false) {
            self.upper.open_dir(handle, path).await
        } else {
            self.lower.open_dir(handle, path).await
        }
    }

    async fn close_dir(&self, handle: &DirHandle) -> VfsResult<()> {
        let _ = self.upper.close_dir(handle).await;
        let _ = self.lower.close_dir(handle).await;
        Ok(())
    }

    async fn read(&self, handle: &FileHandle) -> VfsResult<Vec<u8>> {
        // We don't know which underlying system owns the FileHandle. Try upper, then lower.
        if let Ok(data) = self.upper.read(handle).await {
            return Ok(data);
        }
        self.lower.read(handle).await
    }

    async fn write(&self, handle: &FileHandle, content: &[u8]) -> VfsResult<()> {
        // Writes always go to upper. If it's a lower handle, this is an error
        // since lower files are opened read-only.
        self.upper.write(handle, content).await
    }

    async fn close(&self, handle: &FileHandle) -> VfsResult<()> {
        // Close on both, one will fail but that's fine.
        let _ = self.upper.close(handle).await;
        let _ = self.lower.close(handle).await;
        Ok(())
    }
}
