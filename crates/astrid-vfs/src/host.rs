use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use astrid_capabilities::{DirHandle, FileHandle};
use async_trait::async_trait;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::{RwLock, Semaphore};

use crate::path::resolve_path;
use crate::{Vfs, VfsDirEntry, VfsError, VfsMetadata, VfsResult};

/// An implementation of `Vfs` backed by the physical host filesystem.
pub struct HostVfs {
    open_dirs: RwLock<HashMap<DirHandle, PathBuf>>,
    open_files: RwLock<HashMap<FileHandle, Arc<RwLock<fs::File>>>>,
    fd_semaphore: Arc<Semaphore>,
}

impl HostVfs {
    /// Create a new host VFS.
    #[must_use]
    pub fn new() -> Self {
        Self {
            open_dirs: RwLock::new(HashMap::new()),
            open_files: RwLock::new(HashMap::new()),
            fd_semaphore: Arc::new(Semaphore::new(64)),
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
        let canonical_base = tokio::fs::canonicalize(&base).await.unwrap_or_else(|_| base.clone());
        
        let mut current_check = resolved.clone();
        let mut unexisting_components = Vec::new();

        loop {
            if let Ok(meta) = tokio::fs::symlink_metadata(&current_check).await {
                if meta.is_symlink() {
                    return Err(VfsError::SandboxViolation(
                        "Symlinks are strictly forbidden within the VFS sandbox".into(),
                    ));
                }
                
                let canonical = tokio::fs::canonicalize(&current_check).await.map_err(VfsError::from)?;
                if !canonical.starts_with(&canonical_base) {
                    return Err(VfsError::SandboxViolation(
                        "Path resolves outside sandbox boundaries via symlink".into(),
                    ));
                }
                
                // Construct the secure final path by appending the unexisting components
                // to the canonicalized base path, nullifying any TOCTOU symlink substitution.
                let mut final_path = canonical;
                for comp in unexisting_components.into_iter().rev() {
                    final_path.push(comp);
                }
                return Ok(final_path);
            }
            if let Some(parent) = current_check.parent() {
                if let Some(file_name) = current_check.file_name() {
                    unexisting_components.push(file_name.to_owned());
                }
                current_check = parent.to_path_buf();
            } else {
                break;
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
            let is_dir = match tokio::fs::symlink_metadata(entry.path()).await {
                Ok(meta) => meta.is_dir(),
                Err(_) => false, // Gracefully handle broken symlinks or permission errors
            };
            entries.push(VfsDirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir,
            });
        }
        Ok(entries)
    }

    async fn stat(&self, handle: &DirHandle, path: &str) -> VfsResult<VfsMetadata> {
        let target = self.resolve_physical_path(handle, path).await?;
        let metadata = tokio::fs::metadata(&target).await.map_err(VfsError::from)?;
        
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0u64, |d| d.as_secs());
            
        Ok(VfsMetadata {
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            size: metadata.len(),
            mtime,
        })
    }

    async fn mkdir(&self, handle: &DirHandle, path: &str) -> VfsResult<()> {
        let base = self.get_dir_path(handle).await?;
        let target = self.resolve_physical_path(handle, path).await?;
        if target == base {
            return Err(VfsError::PermissionDenied("Cannot operate on capability root directly".into()));
        }
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(VfsError::from)
    }

    async fn unlink(&self, handle: &DirHandle, path: &str) -> VfsResult<()> {
        let base = self.get_dir_path(handle).await?;
        let target = self.resolve_physical_path(handle, path).await?;
        if target == base {
            return Err(VfsError::PermissionDenied("Cannot operate on capability root directly".into()));
        }
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

    async fn open(&self, handle: &DirHandle, path: &str, write: bool, truncate: bool) -> VfsResult<FileHandle> {
        let target = self.resolve_physical_path(handle, path).await?;

        // Prevent transient FD exhaustion via semaphore before calling the OS
        let permit = self.fd_semaphore.clone().try_acquire_owned().map_err(|_| {
            VfsError::PermissionDenied("Too many open files".into())
        })?;

        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .create(write)
            .truncate(truncate)
            .open(&target)
            .await
            .map_err(VfsError::from)?;

        let mut files = self.open_files.write().await;
        
        // The semaphore guarantees we don't exceed 64, but we leave the map capacity check
        // just as an extra consistency assertion
        if files.len() >= 64 {
            return Err(VfsError::PermissionDenied("Too many open files".into()));
        }

        let new_handle = FileHandle::new();
        files.insert(new_handle.clone(), Arc::new(RwLock::new(file)));

        // Intentionally leak the permit so it is tied to the open handle's lifetime; 
        // it will be returned manually in close()
        permit.forget();

        Ok(new_handle)
    }

    async fn open_dir(&self, handle: &DirHandle, path: &str, new_handle: DirHandle) -> VfsResult<()> {
        let target = self.resolve_physical_path(handle, path).await?;

        {
            let dirs = self.open_dirs.read().await;
            if dirs.len() >= 64 {
                return Err(VfsError::PermissionDenied("Too many open directories".into()));
            }
        }

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
        
        dirs.insert(new_handle.clone(), target);
        Ok(())
    }

    async fn close_dir(&self, handle: &DirHandle) -> VfsResult<()> {
        let mut dirs = self.open_dirs.write().await;
        if dirs.remove(handle).is_none() {
            return Err(VfsError::InvalidHandle);
        }
        Ok(())
    }

    async fn read(&self, handle: &FileHandle) -> VfsResult<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let file_arc = {
            let files = self.open_files.read().await;
            files.get(handle).cloned().ok_or(VfsError::InvalidHandle)?
        };

        let mut file = file_arc.write().await;
        
        let meta = file.metadata().await.map_err(VfsError::from)?;
        let max_size = 50 * 1024 * 1024;
        if meta.len() > max_size as u64 {
            return Err(VfsError::PermissionDenied("File is too large to read into memory (> 50MB)".into()));
        }
        
        let mut buffer = Vec::new();
        let mut file_handle = (&mut *file).take((max_size as u64).saturating_add(1));
        file_handle.read_to_end(&mut buffer)
            .await
            .map_err(VfsError::from)?;
            
        if buffer.len() > max_size {
            return Err(VfsError::PermissionDenied("File grew beyond size limit during read (> 50MB)".into()));
        }
        
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
        self.fd_semaphore.add_permits(1);
        Ok(())
    }
}
