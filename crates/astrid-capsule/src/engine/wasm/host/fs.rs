use extism::{CurrentPlugin, Error, UserData, Val};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

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
fn resolve_physical_absolute(root: &Path, requested: &str) -> Result<ResolvedPhysical, Error> {
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
                return Err(Error::msg(format!(
                    "path escapes root boundary: {requested} resolves to {}",
                    final_path.display()
                )));
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
        return Err(Error::msg(format!(
            "path escapes root boundary: {requested} resolves to {}",
            joined.display()
        )));
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
fn resolve_path(state: &HostState, raw_path: &str) -> Result<ResolvedPath, Error> {
    if let Some(stripped) = raw_path.strip_prefix(CWD_SCHEME) {
        let resolved = resolve_physical_absolute(&state.workspace_root, stripped)?;
        let relative = resolved
            .physical
            .strip_prefix(&resolved.canonical_root)
            .map_err(|_| Error::msg("resolved cwd path escaped canonical root"))?
            .to_path_buf();
        Ok(ResolvedPath {
            physical: resolved.physical,
            relative,
            target: VfsTarget::Workspace,
        })
    } else if let Some(stripped) = raw_path.strip_prefix(HOME_SCHEME) {
        let home_root = state.home_root.as_ref().ok_or_else(|| {
            Error::msg(
                "home:// scheme is not available: no home directory is configured. \
                 Create the directory and restart the kernel.",
            )
        })?;
        let resolved = resolve_physical_absolute(home_root, stripped)?;
        let relative = resolved
            .physical
            .strip_prefix(&resolved.canonical_root)
            .map_err(|_| Error::msg("resolved home path escaped canonical root"))?
            .to_path_buf();
        Ok(ResolvedPath {
            physical: resolved.physical,
            relative,
            target: VfsTarget::Home,
        })
    } else if raw_path.starts_with(TMP_PREFIX) || raw_path == "/tmp" {
        let tmp_root = state.tmp_dir.as_ref().ok_or_else(|| {
            Error::msg("/tmp is not available: no tmp directory is configured for this principal.")
        })?;
        let stripped = raw_path
            .strip_prefix(TMP_PREFIX)
            .or_else(|| raw_path.strip_prefix("/tmp"))
            .unwrap_or("");
        let resolved = resolve_physical_absolute(tmp_root, stripped)?;
        let relative = resolved
            .physical
            .strip_prefix(&resolved.canonical_root)
            .map_err(|_| Error::msg("resolved /tmp path escaped canonical root"))?
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
            .map_err(|_| Error::msg("resolved path escaped canonical root"))?
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
fn resolve_vfs(state: &HostState, resolved: &ResolvedPath) -> Result<ResolvedVfsPath, Error> {
    match resolved.target {
        VfsTarget::Home => {
            let vfs = state.home_vfs.clone().ok_or_else(|| {
                Error::msg(
                    "home:// VFS is not mounted. \
                     Create the directory and restart the kernel.",
                )
            })?;
            let handle = state
                .home_vfs_root_handle
                .clone()
                .ok_or_else(|| Error::msg("home:// VFS root handle is not available"))?;
            Ok(ResolvedVfsPath {
                relative: resolved.relative.clone(),
                vfs,
                handle,
            })
        },
        VfsTarget::Tmp => {
            let vfs = state
                .tmp_vfs
                .clone()
                .ok_or_else(|| Error::msg("/tmp VFS is not mounted for this principal."))?;
            let handle = state
                .tmp_vfs_root_handle
                .clone()
                .ok_or_else(|| Error::msg("/tmp VFS root handle is not available"))?;
            Ok(ResolvedVfsPath {
                relative: resolved.relative.clone(),
                vfs,
                handle,
            })
        },
        VfsTarget::Workspace => Ok(ResolvedVfsPath {
            relative: resolved.relative.clone(),
            vfs: state.vfs.clone(),
            handle: state.vfs_root_handle.clone(),
        }),
    }
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_exists_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let path = String::from_utf8(path_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    // Safety: HostState lock is held across bounded_block_on. This is safe because
    // WASM is single-threaded per plugin - the plugin mutex in invoke_interceptor /
    // run loop serializes all host function calls, so no concurrent lock contention
    // is possible on the same UserData. The lock is needed for resolve_path/resolve_vfs
    // which reference multiple HostState fields.
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let capsule_id = state.capsule_id.as_str().to_owned();

    // Phase 1: resolve to physical path
    let resolved = match resolve_path(&state, &path) {
        Ok(r) => r,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    let security = state.security.clone();
    if let Some(gate) = security {
        let p = resolved.physical.to_string_lossy().to_string();
        let pid = capsule_id.clone();
        let check =
            util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async move {
                gate.check_file_read(&pid, &p).await
            });
        if let Err(reason) = check {
            return util::write_host_result(
                plugin,
                outputs,
                Err(format!("security denied exists check: {reason}")),
            );
        }
    }

    let vfs_path = match resolve_vfs(&state, &resolved) {
        Ok(v) => v,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    let exists = util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async {
        vfs_path
            .vfs
            .exists(
                &vfs_path.handle,
                vfs_path.relative.to_string_lossy().as_ref(),
            )
            .await
    })
    .unwrap_or(false);

    let result = if exists {
        b"true".to_vec()
    } else {
        b"".to_vec()
    };
    util::write_host_result(plugin, outputs, Ok(result))
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_mkdir_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let path = String::from_utf8(path_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_path(&state, &path)?;

    let security = state.security.clone();
    if let Some(gate) = security {
        let p = resolved.physical.to_string_lossy().to_string();
        let pid = capsule_id.clone();
        let check =
            util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async move {
                gate.check_file_write(&pid, &p).await
            });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied mkdir: {reason}")));
        }
    }

    let vfs_path = resolve_vfs(&state, &resolved)?;

    util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async {
        vfs_path
            .vfs
            .mkdir(
                &vfs_path.handle,
                vfs_path.relative.to_string_lossy().as_ref(),
            )
            .await
    })
    .map_err(|e| Error::msg(format!("mkdir failed: {e}")))?;

    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_readdir_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let path = String::from_utf8(path_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = match resolve_path(&state, &path) {
        Ok(r) => r,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    let security = state.security.clone();
    if let Some(gate) = security {
        let p = resolved.physical.to_string_lossy().to_string();
        let pid = capsule_id.clone();
        let check =
            util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async move {
                gate.check_file_read(&pid, &p).await
            });
        if let Err(reason) = check {
            return util::write_host_result(
                plugin,
                outputs,
                Err(format!("security denied readdir: {reason}")),
            );
        }
    }

    let vfs_path = match resolve_vfs(&state, &resolved) {
        Ok(v) => v,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    match util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async {
        vfs_path
            .vfs
            .readdir(
                &vfs_path.handle,
                vfs_path.relative.to_string_lossy().as_ref(),
            )
            .await
    }) {
        Ok(entries) => {
            let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
            let json = serde_json::to_string(&names).unwrap_or_default();
            util::write_host_result(plugin, outputs, Ok(json.into_bytes()))
        },
        Err(e) => util::write_host_result(plugin, outputs, Err(format!("readdir failed: {e}"))),
    }
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_stat_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let path = String::from_utf8(path_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = match resolve_path(&state, &path) {
        Ok(r) => r,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    let security = state.security.clone();
    if let Some(gate) = security {
        let p = resolved.physical.to_string_lossy().to_string();
        let pid = capsule_id.clone();
        let check =
            util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async move {
                gate.check_file_read(&pid, &p).await
            });
        if let Err(reason) = check {
            return util::write_host_result(
                plugin,
                outputs,
                Err(format!("security denied stat: {reason}")),
            );
        }
    }

    let vfs_path = match resolve_vfs(&state, &resolved) {
        Ok(v) => v,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    match util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async {
        vfs_path
            .vfs
            .stat(
                &vfs_path.handle,
                vfs_path.relative.to_string_lossy().as_ref(),
            )
            .await
    }) {
        Ok(metadata) => {
            let json = serde_json::json!({
                "size": metadata.size,
                "isDir": metadata.is_dir,
                "mtime": metadata.mtime
            })
            .to_string();
            util::write_host_result(plugin, outputs, Ok(json.into_bytes()))
        },
        Err(e) => util::write_host_result(plugin, outputs, Err(format!("stat failed: {e}"))),
    }
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_unlink_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let path = String::from_utf8(path_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_path(&state, &path)?;

    let security = state.security.clone();
    if let Some(gate) = security {
        let p = resolved.physical.to_string_lossy().to_string();
        let pid = capsule_id.clone();
        let check =
            util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async move {
                gate.check_file_write(&pid, &p).await
            });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied unlink: {reason}")));
        }
    }

    let vfs_path = resolve_vfs(&state, &resolved)?;

    util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async {
        vfs_path
            .vfs
            .unlink(
                &vfs_path.handle,
                vfs_path.relative.to_string_lossy().as_ref(),
            )
            .await
    })
    .map_err(|e| Error::msg(format!("unlink failed: {e}")))?;

    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_read_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let path = String::from_utf8(path_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = match resolve_path(&state, &path) {
        Ok(r) => r,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    let security = state.security.clone();
    if let Some(gate) = security {
        let p = resolved.physical.to_string_lossy().to_string();
        let pid = capsule_id.clone();
        let check =
            util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async move {
                gate.check_file_read(&pid, &p).await
            });
        if let Err(reason) = check {
            return util::write_host_result(
                plugin,
                outputs,
                Err(format!("security denied read_file: {reason}")),
            );
        }
    }

    let vfs_path = match resolve_vfs(&state, &resolved) {
        Ok(v) => v,
        Err(e) => return util::write_host_result(plugin, outputs, Err(format!("{e}"))),
    };

    let content_result =
        util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async {
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
        });

    match content_result {
        Ok(bytes) => util::write_host_result(plugin, outputs, Ok(bytes)),
        Err(e) => util::write_host_result(plugin, outputs, Err(format!("IO error: {e}"))),
    }
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_write_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let content_bytes: Vec<u8> =
        util::get_safe_bytes(plugin, &inputs[1], util::MAX_GUEST_PAYLOAD_LEN)?;
    let path = String::from_utf8(path_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_path(&state, &path)?;

    let security = state.security.clone();
    if let Some(gate) = security {
        let p = resolved.physical.to_string_lossy().to_string();
        let pid = capsule_id.clone();
        let check =
            util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async move {
                gate.check_file_write(&pid, &p).await
            });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied write_file: {reason}")));
        }
    }

    let vfs_path = resolve_vfs(&state, &resolved)?;

    util::bounded_block_on(&state.runtime_handle, &state.host_semaphore, async {
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
        let res = vfs_path.vfs.write(&handle, &content_bytes).await;
        let _ = vfs_path.vfs.close(&handle).await;
        res
    })
    .map_err(|e| Error::msg(format!("write_file failed: {e}")))?;

    Ok(())
}
