use std::io::Read;
use std::path::{Component, Path, PathBuf};

use extism::{CurrentPlugin, Error, UserData, Val};

use cap_std::fs::Dir;

use crate::wasm::host::util;
use crate::wasm::host_state::HostState;

/// Maximum number of directory entries allowed to be read by `readdir`
const MAX_READDIR_ENTRIES: usize = 10_000;

/// Strip any leading absolute slashes or prefixes (e.g. C:\) from the requested path
/// so that `cap_std` can open it relative to the directory capability.
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
/// Returns both the absolute path and a sanitized relative path safe for capability use.
/// This prevents symlink bypass attacks where a lexical path passes the gate but cap-std follows a symlink.
fn resolve_physical_absolute(
    workspace_root: &Path,
    requested: &str,
) -> Result<(PathBuf, PathBuf), Error> {
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let relative_requested = make_relative(requested);
    let joined = canonical_root.join(relative_requested);

    // Find the deepest existing ancestor to canonicalize to resolve any symlinks
    let mut current = joined.clone();
    let mut non_existent_components = Vec::new();

    while !current.exists() {
        if let Some(parent) = current.parent() {
            if let Some(file_name) = current.file_name() {
                non_existent_components.push(file_name.to_os_string());
            }
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    // Canonicalize the existing part to resolve symlinks
    let mut resolved = current.canonicalize().unwrap_or_else(|_| current.clone());

    // Re-attach the non-existent components, applying `..` and `.` lexically
    for comp in non_existent_components.into_iter().rev() {
        if comp == ".." {
            resolved.pop();
        } else if comp != "." {
            resolved.push(comp);
        }
    }

    if !resolved.starts_with(&canonical_root) {
        return Err(Error::msg(format!(
            "path escapes workspace boundary: {requested} resolves to {}",
            resolved.display()
        )));
    }

    let mut safe_relative = resolved
        .strip_prefix(&canonical_root)
        .unwrap_or(Path::new(""))
        .to_path_buf();
    if safe_relative.as_os_str().is_empty() {
        safe_relative = PathBuf::from(".");
    }

    Ok((resolved, safe_relative))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_exists_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = util::get_safe_string(plugin, &inputs[0], util::MAX_PATH_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let workspace_root = state.workspace_root.clone();
    drop(state);

    let (_, safe_relative) = resolve_physical_absolute(&workspace_root, &path)?;

    let dir = Dir::open_ambient_dir(&workspace_root, cap_std::ambient_authority())
        .map_err(|e| Error::msg(format!("failed to open workspace dir: {e}")))?;

    let exists = dir.exists(&safe_relative);

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
    let path: String = util::get_safe_string(plugin, &inputs[0], util::MAX_PATH_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let (absolute, safe_relative) = resolve_physical_absolute(&workspace_root, &path)?;
    let absolute_str = absolute.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let astr = absolute_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &astr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied mkdir: {reason}")));
        }
    }

    let dir = Dir::open_ambient_dir(&workspace_root, cap_std::ambient_authority())
        .map_err(|e| Error::msg(format!("failed to open workspace dir: {e}")))?;

    dir.create_dir_all(&safe_relative)
        .map_err(|e| Error::msg(format!("mkdir failed ({absolute_str}): {e}")))?;

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_fs_readdir_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = util::get_safe_string(plugin, &inputs[0], util::MAX_PATH_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let (absolute, safe_relative) = resolve_physical_absolute(&workspace_root, &path)?;
    let absolute_str = absolute.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let astr = absolute_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &astr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied readdir: {reason}")));
        }
    }

    let dir = Dir::open_ambient_dir(&workspace_root, cap_std::ambient_authority())
        .map_err(|e| Error::msg(format!("failed to open workspace dir: {e}")))?;

    let iter = dir
        .read_dir(&safe_relative)
        .map_err(|e| Error::msg(format!("readdir failed ({absolute_str}): {e}")))?;

    let mut entries = Vec::new();
    for (count, entry_res) in iter.enumerate() {
        if count >= MAX_READDIR_ENTRIES {
            return Err(Error::msg(format!(
                "directory listing exceeds maximum entries limit ({MAX_READDIR_ENTRIES})"
            )));
        }
        match entry_res {
            Ok(entry) => {
                if let Ok(name) = entry.file_name().into_string() {
                    entries.push(name);
                }
            },
            Err(e) => {
                tracing::warn!(plugin = %plugin_id, "readdir entry error: {e}");
            },
        }
    }

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
    let path: String = util::get_safe_string(plugin, &inputs[0], util::MAX_PATH_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let (absolute, safe_relative) = resolve_physical_absolute(&workspace_root, &path)?;
    let absolute_str = absolute.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let astr = absolute_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &astr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied stat: {reason}")));
        }
    }

    let dir = Dir::open_ambient_dir(&workspace_root, cap_std::ambient_authority())
        .map_err(|e| Error::msg(format!("failed to open workspace dir: {e}")))?;

    let metadata = dir
        .metadata(&safe_relative)
        .map_err(|e| Error::msg(format!("stat failed ({absolute_str}): {e}")))?;

    let mtime = metadata
        .modified()
        .ok()
        .map(cap_std::time::SystemTime::into_std)
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
    let path: String = util::get_safe_string(plugin, &inputs[0], util::MAX_PATH_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let (absolute, safe_relative) = resolve_physical_absolute(&workspace_root, &path)?;
    let absolute_str = absolute.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let astr = absolute_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &astr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied unlink: {reason}")));
        }
    }

    let dir = Dir::open_ambient_dir(&workspace_root, cap_std::ambient_authority())
        .map_err(|e| Error::msg(format!("failed to open workspace dir: {e}")))?;

    dir.remove_file(&safe_relative)
        .map_err(|e| Error::msg(format!("unlink failed ({absolute_str}): {e}")))?;

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_read_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = util::get_safe_string(plugin, &inputs[0], util::MAX_PATH_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let (absolute, safe_relative) = resolve_physical_absolute(&workspace_root, &path)?;
    let absolute_str = absolute.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let astr = absolute_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &astr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file read: {reason}")));
        }
    }

    let dir = Dir::open_ambient_dir(&workspace_root, cap_std::ambient_authority())
        .map_err(|e| Error::msg(format!("failed to open workspace dir: {e}")))?;

    let file = dir
        .open(&safe_relative)
        .map_err(|e| Error::msg(format!("failed to open file ({absolute_str}): {e}")))?;

    let mut bytes = Vec::new();
    file.take(util::MAX_GUEST_PAYLOAD_LEN + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::msg(format!("read_file failed ({absolute_str}): {e}")))?;

    if bytes.len() as u64 > util::MAX_GUEST_PAYLOAD_LEN {
        return Err(Error::msg(
            "file size exceeds maximum allowed guest payload limit",
        ));
    }

    let content = String::from_utf8(bytes)
        .map_err(|e| Error::msg(format!("file content is not valid UTF-8: {e}")))?;

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
    let path: String = util::get_safe_string(plugin, &inputs[0], util::MAX_PATH_LEN)?;
    let content: String = util::get_safe_string(plugin, &inputs[1], util::MAX_GUEST_PAYLOAD_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let (absolute, safe_relative) = resolve_physical_absolute(&workspace_root, &path)?;
    let absolute_str = absolute.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let astr = absolute_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &astr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file write: {reason}")));
        }
    }

    let dir = Dir::open_ambient_dir(&workspace_root, cap_std::ambient_authority())
        .map_err(|e| Error::msg(format!("failed to open workspace dir: {e}")))?;

    dir.write(&safe_relative, content.as_bytes())
        .map_err(|e| Error::msg(format!("write_file failed ({absolute_str}): {e}")))?;

    Ok(())
}
