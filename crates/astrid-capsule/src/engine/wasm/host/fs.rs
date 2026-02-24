use extism::{CurrentPlugin, Error, UserData, Val};
use std::path::{Component, Path, PathBuf};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

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

/// Compute the true physical absolute path for the security gate by canonicalizing on the host filesystem.
/// This prevents symlink bypass attacks where a lexical path passes the gate but cap-std follows a symlink.
fn resolve_physical_absolute(workspace_root: &Path, requested: &str) -> Result<PathBuf, Error> {
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

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
                    "path escapes workspace boundary: {requested} resolves to {}",
                    final_path.display()
                )));
            }
            return Ok(final_path);
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
            "path escapes workspace boundary: {requested} resolves to {}",
            joined.display()
        )));
    }

    Ok(joined)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_exists_impl(
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

    let _capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_physical_absolute(&state.workspace_root, &path)?;

    let security = state.security.clone();

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = _capsule_id.clone();
        let check = tokio::task::block_in_place(|| {
            state
                .runtime_handle
                .block_on(async move { gate.check_file_read(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied exists check: {reason}"
            )));
        }
    }

    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    let safe_relative = resolved.strip_prefix(&canonical_root).map_err(|_| Error::msg("resolved path escaped canonical root"))?;

    // We allow read checks natively, but we ensure it uses the resolved path
    let exists = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state
                .vfs
                .exists(
                    &state.vfs_root_handle,
                    safe_relative.to_string_lossy().as_ref(),
                )
                .await
        })
    })
    .unwrap_or(false);

    let result = if exists {
        b"true".to_vec()
    } else {
        b"".to_vec()
    };
    let mem = plugin.memory_new(result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
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
    let _capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_physical_absolute(&state.workspace_root, &path)?;

    let security = state.security.clone();

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = _capsule_id.clone();
        let check = tokio::task::block_in_place(|| {
            state
                .runtime_handle
                .block_on(async move { gate.check_file_write(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied mkdir: {reason}")));
        }
    }

    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    let safe_relative = resolved.strip_prefix(&canonical_root).map_err(|_| Error::msg("resolved path escaped canonical root"))?;

    tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state
                .vfs
                .mkdir(
                    &state.vfs_root_handle,
                    safe_relative.to_string_lossy().as_ref(),
                )
                .await
        })
    })
    .map_err(|e| Error::msg(format!("mkdir failed: {e}")))?;

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
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
    let _capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_physical_absolute(&state.workspace_root, &path)?;

    let security = state.security.clone();

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = _capsule_id.clone();
        let check = tokio::task::block_in_place(|| {
            state
                .runtime_handle
                .block_on(async move { gate.check_file_read(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied readdir: {reason}")));
        }
    }

    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    let safe_relative = resolved.strip_prefix(&canonical_root).map_err(|_| Error::msg("resolved path escaped canonical root"))?;

    let entries = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state
                .vfs
                .readdir(
                    &state.vfs_root_handle,
                    safe_relative.to_string_lossy().as_ref(),
                )
                .await
        })
    })
    .map_err(|e| Error::msg(format!("readdir failed: {e}")))?;

    // We historically map this to an array of strings in extism
    let string_entries: Vec<String> = entries.into_iter().map(|e| e.name).collect();

    let json = serde_json::to_string(&string_entries)
        .map_err(|e| Error::msg(format!("failed to serialize directory entries: {e}")))?;

    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
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

    let _capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_physical_absolute(&state.workspace_root, &path)?;

    let security = state.security.clone();

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = _capsule_id.clone();
        let check = tokio::task::block_in_place(|| {
            state
                .runtime_handle
                .block_on(async move { gate.check_file_read(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied stat: {reason}")));
        }
    }

    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    let safe_relative = resolved.strip_prefix(&canonical_root).map_err(|_| Error::msg("resolved path escaped canonical root"))?;

    let metadata = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state
                .vfs
                .stat(
                    &state.vfs_root_handle,
                    safe_relative.to_string_lossy().as_ref(),
                )
                .await
        })
    })
    .map_err(|e| Error::msg(format!("stat failed: {e}")))?;

    let stat = serde_json::json!({
        "size": metadata.size,
        "isDir": metadata.is_dir,
        "mtime": metadata.mtime
    });

    let json = stat.to_string();
    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
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

    let _capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_physical_absolute(&state.workspace_root, &path)?;

    let security = state.security.clone();

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = _capsule_id.clone();
        let check = tokio::task::block_in_place(|| {
            state
                .runtime_handle
                .block_on(async move { gate.check_file_write(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied unlink: {reason}")));
        }
    }

    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    let safe_relative = resolved.strip_prefix(&canonical_root).map_err(|_| Error::msg("resolved path escaped canonical root"))?;

    tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state
                .vfs
                .unlink(
                    &state.vfs_root_handle,
                    safe_relative.to_string_lossy().as_ref(),
                )
                .await
        })
    })
    .map_err(|e| Error::msg(format!("unlink failed: {e}")))?;

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
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

    let _capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_physical_absolute(&state.workspace_root, &path)?;

    let security = state.security.clone();

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = _capsule_id.clone();
        let check = tokio::task::block_in_place(|| {
            state
                .runtime_handle
                .block_on(async move { gate.check_file_read(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied read_file: {reason}")));
        }
    }

    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    let safe_relative = resolved.strip_prefix(&canonical_root).map_err(|_| Error::msg("resolved path escaped canonical root"))?;

    let content_bytes = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            let metadata = state.vfs.stat(&state.vfs_root_handle, safe_relative.to_string_lossy().as_ref()).await?;
            if metadata.size > util::MAX_GUEST_PAYLOAD_LEN {
                return Err(astrid_vfs::VfsError::PermissionDenied(format!(
                    "File too large to read into memory ({} bytes > {} bytes)",
                    metadata.size,
                    util::MAX_GUEST_PAYLOAD_LEN
                )));
            }

            let handle = state
                .vfs
                .open(
                    &state.vfs_root_handle,
                    safe_relative.to_string_lossy().as_ref(),
                    false,
                    false,
                )
                .await?;
            let data = state.vfs.read(&handle).await;
            let _ = state.vfs.close(&handle).await;
            data
        })
    })
    .map_err(|e| Error::msg(format!("read_file failed: {e}")))?;

    let mem = plugin.memory_new(&content_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
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

    let _capsule_id = state.capsule_id.as_str().to_owned();

    let resolved = resolve_physical_absolute(&state.workspace_root, &path)?;

    let security = state.security.clone();

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = _capsule_id.clone();
        let check = tokio::task::block_in_place(|| {
            state
                .runtime_handle
                .block_on(async move { gate.check_file_write(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied write_file: {reason}")));
        }
    }

    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    let safe_relative = resolved.strip_prefix(&canonical_root).map_err(|_| Error::msg("resolved path escaped canonical root"))?;

    tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            // Note: pass truncate=true to emulate standard write behavior
            let handle = state
                .vfs
                .open(
                    &state.vfs_root_handle,
                    safe_relative.to_string_lossy().as_ref(),
                    true,
                    true,
                )
                .await?;
            let res = state.vfs.write(&handle, &content_bytes).await;
            let _ = state.vfs.close(&handle).await;
            res
        })
    })
    .map_err(|e| Error::msg(format!("write_file failed: {e}")))?;

    Ok(())
}
