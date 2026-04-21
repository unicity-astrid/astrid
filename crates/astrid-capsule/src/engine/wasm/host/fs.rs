use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::engine::wasm::bindings::astrid::capsule::fs;
use crate::engine::wasm::bindings::astrid::capsule::types::FileStat;
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

/// URI scheme prefix for the principal's home directory.
const HOME_SCHEME: &str = "home://";

/// URI scheme prefix for the daemon's current working directory.
const CWD_SCHEME: &str = "cwd://";

/// Path prefix that maps to the principal's tmp directory.
const TMP_PREFIX: &str = "/tmp/";

/// Strip any leading absolute slashes or prefixes (e.g. C:\) from the requested path
fn make_relative(requested: &str) -> &Path {
    let path = Path::new(requested);
    let mut components = path.components();
    while let Some(c) = components.clone().next() {
        if matches!(c, Component::RootDir | Component::Prefix(_)) {
            components.next(); // consume it
        } else {
            break;
        }
    }
    components.as_path()
}

/// Result of resolving a path to a physical absolute location on disk.
struct ResolvedPhysical {
    /// The fully resolved physical path (symlinks canonicalized where possible).
    physical: PathBuf,
    /// The canonical root this path was resolved against.
    canonical_root: PathBuf,
}

/// Compute the true physical absolute path for the security gate by canonicalizing on the host filesystem.
/// This prevents symlink bypass attacks where a lexical path passes the gate but cap-std follows a symlink.
fn resolve_physical_absolute(root: &Path, requested: &str) -> Result<ResolvedPhysical, String> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let relative_requested = make_relative(requested);
    let joined = canonical_root.join(relative_requested);

    let mut current_check = joined.clone();
    let mut unexisting_components = Vec::new();

    loop {
        if std::fs::symlink_metadata(&current_check).is_ok() {
            let canonical =
                std::fs::canonicalize(&current_check).unwrap_or_else(|_| current_check.clone());
            let mut final_path = canonical;
            for comp in unexisting_components.into_iter().rev() {
                final_path.push(comp);
            }
            if !final_path.starts_with(&canonical_root) {
                return Err(format!(
                    "path escapes root boundary: {requested} resolves to {}",
                    final_path.display()
                ));
            }
            return Ok(ResolvedPhysical {
                physical: final_path,
                canonical_root,
            });
        }
        if let Some(parent) = current_check.parent() {
            if let Some(file_name) = current_check.file_name() {
                unexisting_components.push(file_name.to_os_string());
            }
            current_check = parent.to_path_buf();
        } else {
            break;
        }
    }

    if !joined.starts_with(&canonical_root) {
        return Err(format!(
            "path escapes root boundary: {requested} resolves to {}",
            joined.display()
        ));
    }

    Ok(ResolvedPhysical {
        physical: joined,
        canonical_root,
    })
}

/// Which VFS target a resolved path points at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VfsTarget {
    /// The workspace overlay VFS (default).
    Workspace,
    /// The principal's home directory (`home://`).
    Home,
    /// The principal's tmp directory (`/tmp/`).
    Tmp,
}

/// First-phase resolution result: physical path for the security gate,
/// the VFS-relative path, and which VFS to target.
struct ResolvedPath {
    /// Absolute physical path (for security gate check).
    physical: PathBuf,
    /// Path relative to the root (for VFS operations).
    relative: PathBuf,
    /// Which VFS this path targets.
    target: VfsTarget,
}

/// Second-phase resolution result: the VFS instance and capability handle
/// to use for the actual filesystem operation.
struct ResolvedVfsPath {
    /// Path relative to the VFS root.
    relative: PathBuf,
    /// The VFS instance to use.
    vfs: Arc<dyn astrid_vfs::Vfs>,
    /// The capability handle for the VFS root.
    handle: astrid_capabilities::DirHandle,
}

/// Phase 1: Resolve a raw guest path to a physical path and determine
/// whether it targets the workspace or home VFS.
///
/// Uses the *effective* home root and tmp dir, which prefer the per-invocation
/// principal's paths over the capsule's load-time paths when set by
/// `WasmEngine::invoke_interceptor`.
fn resolve_path(state: &HostState, raw_path: &str) -> Result<ResolvedPath, String> {
    if let Some(stripped) = raw_path.strip_prefix(CWD_SCHEME) {
        let resolved = resolve_physical_absolute(&state.workspace_root, stripped)?;
        let relative = resolved
            .physical
            .strip_prefix(&resolved.canonical_root)
            .map_err(|_| "resolved cwd path escaped canonical root".to_string())?
            .to_path_buf();
        Ok(ResolvedPath {
            physical: resolved.physical,
            relative,
            target: VfsTarget::Workspace,
        })
    } else if let Some(stripped) = raw_path.strip_prefix(HOME_SCHEME) {
        let home = state.effective_home().ok_or_else(|| {
            "home:// scheme is not available: no home directory is configured \
             for the calling principal."
                .to_string()
        })?;
        let resolved = resolve_physical_absolute(&home.root, stripped)?;
        let relative = resolved
            .physical
            .strip_prefix(&resolved.canonical_root)
            .map_err(|_| "resolved home path escaped canonical root".to_string())?
            .to_path_buf();
        Ok(ResolvedPath {
            physical: resolved.physical,
            relative,
            target: VfsTarget::Home,
        })
    } else if raw_path.starts_with(TMP_PREFIX) || raw_path == "/tmp" {
        let tmp_mount = state.effective_tmp().ok_or_else(|| {
            "/tmp is not available: no tmp directory is configured for this principal.".to_string()
        })?;
        let stripped = raw_path
            .strip_prefix(TMP_PREFIX)
            .or_else(|| raw_path.strip_prefix("/tmp"))
            .unwrap_or("");
        let resolved = resolve_physical_absolute(&tmp_mount.root, stripped)?;
        let relative = resolved
            .physical
            .strip_prefix(&resolved.canonical_root)
            .map_err(|_| "resolved /tmp path escaped canonical root".to_string())?
            .to_path_buf();
        Ok(ResolvedPath {
            physical: resolved.physical,
            relative,
            target: VfsTarget::Tmp,
        })
    } else {
        let resolved = resolve_physical_absolute(&state.workspace_root, raw_path)?;
        let relative = resolved
            .physical
            .strip_prefix(&resolved.canonical_root)
            .map_err(|_| "resolved path escaped canonical root".to_string())?
            .to_path_buf();
        Ok(ResolvedPath {
            physical: resolved.physical,
            relative,
            target: VfsTarget::Workspace,
        })
    }
}

/// Phase 2: Given a first-phase result, select the correct VFS instance
/// and capability handle.
///
/// For `Home` and `Tmp` targets, returns the *effective* bundle — the
/// per-invocation bundle if `WasmEngine::invoke_interceptor` installed one,
/// otherwise the capsule's load-time bundle.
fn resolve_vfs(state: &HostState, resolved: &ResolvedPath) -> Result<ResolvedVfsPath, String> {
    let (vfs, handle) = match resolved.target {
        VfsTarget::Home => {
            let m = state.effective_home().ok_or_else(|| {
                "home:// VFS is not mounted for the calling principal.".to_string()
            })?;
            (m.vfs.clone(), m.handle.clone())
        },
        VfsTarget::Tmp => {
            let m = state
                .effective_tmp()
                .ok_or_else(|| "/tmp VFS is not mounted for the calling principal.".to_string())?;
            (m.vfs.clone(), m.handle.clone())
        },
        VfsTarget::Workspace => (state.vfs.clone(), state.vfs_root_handle.clone()),
    };
    Ok(ResolvedVfsPath {
        relative: resolved.relative.clone(),
        vfs,
        handle,
    })
}

impl fs::Host for HostState {
    fn fs_exists(&mut self, path: String) -> Result<bool, String> {
        let capsule_id = self.capsule_id.as_str().to_owned();

        // Phase 1: resolve to physical path
        let resolved = resolve_path(self, &path)?;

        let security = self.security.clone();
        if let Some(gate) = security {
            let p = resolved.physical.to_string_lossy().to_string();
            let pid = capsule_id.clone();
            let home = self.effective_home_root_buf();
            let check =
                util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async move {
                    gate.check_file_read(&pid, &p, home.as_deref()).await
                });
            if let Err(reason) = check {
                return Err(format!("security denied exists check: {reason}"));
            }
        }

        let vfs_path = resolve_vfs(self, &resolved)?;

        let exists = util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            vfs_path
                .vfs
                .exists(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                )
                .await
        })
        .unwrap_or(false);

        Ok(exists)
    }

    fn fs_mkdir(&mut self, path: String) -> Result<(), String> {
        let capsule_id = self.capsule_id.as_str().to_owned();

        let resolved = resolve_path(self, &path)?;

        let security = self.security.clone();
        if let Some(gate) = security {
            let p = resolved.physical.to_string_lossy().to_string();
            let pid = capsule_id.clone();
            let home = self.effective_home_root_buf();
            let check =
                util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async move {
                    gate.check_file_write(&pid, &p, home.as_deref()).await
                });
            if let Err(reason) = check {
                return Err(format!("security denied mkdir: {reason}"));
            }
        }

        let vfs_path = resolve_vfs(self, &resolved)?;

        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            vfs_path
                .vfs
                .mkdir(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                )
                .await
        })
        .map_err(|e| format!("mkdir failed: {e}"))
    }

    fn fs_readdir(&mut self, path: String) -> Result<Vec<String>, String> {
        let capsule_id = self.capsule_id.as_str().to_owned();

        let resolved = resolve_path(self, &path)?;

        let security = self.security.clone();
        if let Some(gate) = security {
            let p = resolved.physical.to_string_lossy().to_string();
            let pid = capsule_id.clone();
            let home = self.effective_home_root_buf();
            let check =
                util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async move {
                    gate.check_file_read(&pid, &p, home.as_deref()).await
                });
            if let Err(reason) = check {
                return Err(format!("security denied readdir: {reason}"));
            }
        }

        let vfs_path = resolve_vfs(self, &resolved)?;

        let entries = util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            vfs_path
                .vfs
                .readdir(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                )
                .await
        })
        .map_err(|e| format!("readdir failed: {e}"))?;

        Ok(entries.into_iter().map(|e| e.name).collect())
    }

    fn fs_stat(&mut self, path: String) -> Result<FileStat, String> {
        let capsule_id = self.capsule_id.as_str().to_owned();

        let resolved = resolve_path(self, &path)?;

        let security = self.security.clone();
        if let Some(gate) = security {
            let p = resolved.physical.to_string_lossy().to_string();
            let pid = capsule_id.clone();
            let home = self.effective_home_root_buf();
            let check =
                util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async move {
                    gate.check_file_read(&pid, &p, home.as_deref()).await
                });
            if let Err(reason) = check {
                return Err(format!("security denied stat: {reason}"));
            }
        }

        let vfs_path = resolve_vfs(self, &resolved)?;

        let metadata = util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            vfs_path
                .vfs
                .stat(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                )
                .await
        })
        .map_err(|e| format!("stat failed: {e}"))?;

        Ok(FileStat {
            size: metadata.size,
            is_dir: metadata.is_dir,
            mtime: Some(metadata.mtime),
        })
    }

    fn fs_unlink(&mut self, path: String) -> Result<(), String> {
        let capsule_id = self.capsule_id.as_str().to_owned();

        let resolved = resolve_path(self, &path)?;

        let security = self.security.clone();
        if let Some(gate) = security {
            let p = resolved.physical.to_string_lossy().to_string();
            let pid = capsule_id.clone();
            let home = self.effective_home_root_buf();
            let check =
                util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async move {
                    gate.check_file_write(&pid, &p, home.as_deref()).await
                });
            if let Err(reason) = check {
                return Err(format!("security denied unlink: {reason}"));
            }
        }

        let vfs_path = resolve_vfs(self, &resolved)?;

        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            vfs_path
                .vfs
                .unlink(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                )
                .await
        })
        .map_err(|e| format!("unlink failed: {e}"))
    }

    fn read_file(&mut self, path: String) -> Result<Vec<u8>, String> {
        let capsule_id = self.capsule_id.as_str().to_owned();

        let resolved = resolve_path(self, &path)?;

        let security = self.security.clone();
        if let Some(gate) = security {
            let p = resolved.physical.to_string_lossy().to_string();
            let pid = capsule_id.clone();
            let home = self.effective_home_root_buf();
            let check =
                util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async move {
                    gate.check_file_read(&pid, &p, home.as_deref()).await
                });
            if let Err(reason) = check {
                return Err(format!("security denied read_file: {reason}"));
            }
        }

        let vfs_path = resolve_vfs(self, &resolved)?;

        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            let metadata = vfs_path
                .vfs
                .stat(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                )
                .await?;
            if metadata.size > util::MAX_GUEST_PAYLOAD_LEN {
                return Err(astrid_vfs::VfsError::PermissionDenied(format!(
                    "File too large to read into memory ({} bytes > {} bytes)",
                    metadata.size,
                    util::MAX_GUEST_PAYLOAD_LEN
                )));
            }

            let handle = vfs_path
                .vfs
                .open(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                    false,
                    false,
                )
                .await?;
            let data = vfs_path.vfs.read(&handle).await;
            let _ = vfs_path.vfs.close(&handle).await;
            data
        })
        .map_err(|e| format!("IO error: {e}"))
    }

    fn write_file(&mut self, path: String, content: Vec<u8>) -> Result<(), String> {
        let capsule_id = self.capsule_id.as_str().to_owned();

        let resolved = resolve_path(self, &path)?;

        let security = self.security.clone();
        if let Some(gate) = security {
            let p = resolved.physical.to_string_lossy().to_string();
            let pid = capsule_id.clone();
            let home = self.effective_home_root_buf();
            let check =
                util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async move {
                    gate.check_file_write(&pid, &p, home.as_deref()).await
                });
            if let Err(reason) = check {
                return Err(format!("security denied write_file: {reason}"));
            }
        }

        let vfs_path = resolve_vfs(self, &resolved)?;

        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            // Note: pass truncate=true to emulate standard write behavior
            let handle = vfs_path
                .vfs
                .open(
                    &vfs_path.handle,
                    vfs_path.relative.to_string_lossy().as_ref(),
                    true,
                    true,
                )
                .await?;
            let res = vfs_path.vfs.write(&handle, &content).await;
            let _ = vfs_path.vfs.close(&handle).await;
            res
        })
        .map_err(|e| format!("write_file failed: {e}"))
    }
}
