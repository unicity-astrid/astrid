use std::path::{Path, PathBuf};

use extism::{CurrentPlugin, Error, UserData, Val};

use crate::wasm::host_state::HostState;

/// Lexically normalize a path (resolve `.` and `..` without filesystem access).
fn lexical_normalize(path: &Path) -> Result<PathBuf, Error> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if let Some(last) = components.last() {
                    if matches!(last, std::path::Component::RootDir | std::path::Component::Prefix(_)) {
                        return Err(Error::msg("path traversal attempts to escape root"));
                    }
                    components.pop();
                } else {
                    return Err(Error::msg("path traversal attempts to escape root"));
                }
            },
            std::path::Component::CurDir => {},
            other => components.push(other),
        }
    }
    Ok(components.iter().collect())
}

/// Resolve a plugin-provided path relative to the workspace root and verify
/// it does not escape the workspace boundary.
pub(crate) fn resolve_within_workspace(
    workspace_root: &Path,
    requested: &str,
) -> Result<PathBuf, Error> {
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let requested_path = Path::new(requested);
    let joined = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        canonical_root.join(requested_path)
    };

    let canonical_path = if joined.exists() {
        joined
            .canonicalize()
            .map_err(|e| Error::msg(format!("failed to resolve path: {e}")))?
    } else {
        let parent = joined.parent().unwrap_or(&joined);
        let filename = joined.file_name();
        if parent.exists() {
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| Error::msg(format!("failed to resolve parent: {e}")))?;
            match filename {
                Some(name) => canonical_parent.join(name),
                None => canonical_parent,
            }
        } else {
            lexical_normalize(&joined)?
        }
    };

    if !canonical_path.starts_with(&canonical_root) {
        return Err(Error::msg(format!(
            "path escapes workspace boundary: {requested} resolves to {}",
            canonical_path.display()
        )));
    }

    Ok(canonical_path)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_exists_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let workspace_root = state.workspace_root.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let exists = resolved.exists();

    let result = if exists { "true" } else { "false" };
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
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied mkdir: {reason}")));
        }
    }

    std::fs::create_dir_all(&resolved)
        .map_err(|e| Error::msg(format!("mkdir failed ({resolved_str}): {e}")))?;

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_readdir_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied readdir: {reason}")));
        }
    }

    let entries: Vec<String> = std::fs::read_dir(&resolved)
        .map_err(|e| Error::msg(format!("readdir failed ({resolved_str}): {e}")))?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();

    let json = serde_json::to_string(&entries)
        .map_err(|e| Error::msg(format!("failed to serialize readdir result: {e}")))?;

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
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied stat: {reason}")));
        }
    }

    let metadata = std::fs::metadata(&resolved)
        .map_err(|e| Error::msg(format!("stat failed ({resolved_str}): {e}")))?;

    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0u64, |d| d.as_secs());

    let stat = serde_json::json!({
        "size": metadata.len(),
        "isDir": metadata.is_dir(),
        "mtime": mtime
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
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied unlink: {reason}")));
        }
    }

    std::fs::remove_file(&resolved)
        .map_err(|e| Error::msg(format!("unlink failed ({resolved_str}): {e}")))?;

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_read_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file read: {reason}")));
        }
    }

    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| Error::msg(format!("read_file failed ({resolved_str}): {e}")))?;

    let mem = plugin.memory_new(&content)?;
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
    let path: String = plugin.memory_get_val(&inputs[0])?;
    let content: String = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file write: {reason}")));
        }
    }

    std::fs::write(&resolved, content.as_bytes())
        .map_err(|e| Error::msg(format!("write_file failed ({resolved_str}): {e}")))?;

    Ok(())
}
