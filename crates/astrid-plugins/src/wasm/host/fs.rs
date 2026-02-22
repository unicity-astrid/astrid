use extism::{CurrentPlugin, Error, UserData, Val};

use crate::wasm::host_state::HostState;

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

    let exists = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state.vfs.exists(&state.vfs_root_handle, &path).await
        })
    }).unwrap_or(false);

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
    let security = state.security.clone();
    
    let resolved = astrid_vfs::path::resolve_path(&state.workspace_root, &path)
        .map_err(|e| Error::msg(format!("path resolution failed: {e}")))?;
    
    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = plugin_id.clone();
        let check = tokio::task::block_in_place(|| {
            state.runtime_handle.block_on(async move { gate.check_file_write(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied mkdir: {reason}")));
        }
    }

    tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state.vfs.mkdir(&state.vfs_root_handle, &path).await
        })
    }).map_err(|e| Error::msg(format!("mkdir failed: {e}")))?;

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
    let security = state.security.clone();

    let resolved = astrid_vfs::path::resolve_path(&state.workspace_root, &path)
        .map_err(|e| Error::msg(format!("path resolution failed: {e}")))?;

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = plugin_id.clone();
        let check = tokio::task::block_in_place(|| {
            state.runtime_handle.block_on(async move { gate.check_file_read(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied readdir: {reason}")));
        }
    }

    let entries = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state.vfs.readdir(&state.vfs_root_handle, &path).await
        })
    }).map_err(|e| Error::msg(format!("readdir failed: {e}")))?;

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
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    
    let plugin_id = state.plugin_id.as_str().to_owned();
    let security = state.security.clone();

    let resolved = astrid_vfs::path::resolve_path(&state.workspace_root, &path)
        .map_err(|e| Error::msg(format!("path resolution failed: {e}")))?;

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = plugin_id.clone();
        let check = tokio::task::block_in_place(|| {
            state.runtime_handle.block_on(async move { gate.check_file_read(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied stat: {reason}")));
        }
    }

    let metadata = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state.vfs.stat(&state.vfs_root_handle, &path).await
        })
    }).map_err(|e| Error::msg(format!("stat failed: {e}")))?;

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
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    
    let plugin_id = state.plugin_id.as_str().to_owned();
    let security = state.security.clone();

    let resolved = astrid_vfs::path::resolve_path(&state.workspace_root, &path)
        .map_err(|e| Error::msg(format!("path resolution failed: {e}")))?;

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = plugin_id.clone();
        let check = tokio::task::block_in_place(|| {
            state.runtime_handle.block_on(async move { gate.check_file_write(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied unlink: {reason}")));
        }
    }

    tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            state.vfs.unlink(&state.vfs_root_handle, &path).await
        })
    }).map_err(|e| Error::msg(format!("unlink failed: {e}")))?;

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
    let security = state.security.clone();

    let resolved = astrid_vfs::path::resolve_path(&state.workspace_root, &path)
        .map_err(|e| Error::msg(format!("path resolution failed: {e}")))?;

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = plugin_id.clone();
        let check = tokio::task::block_in_place(|| {
            state.runtime_handle.block_on(async move { gate.check_file_read(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied read_file: {reason}")));
        }
    }

    let content_bytes = tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            let handle = state.vfs.open(&state.vfs_root_handle, &path, false, false).await?;
            let data = state.vfs.read(&handle).await;
            let _ = state.vfs.close(&handle).await;
            data
        })
    }).map_err(|e| Error::msg(format!("read_file failed: {e}")))?;

    let content = String::from_utf8(content_bytes)
        .map_err(|e| Error::msg(format!("failed to parse file content as utf8: {e}")))?;

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
    let security = state.security.clone();

    let resolved = astrid_vfs::path::resolve_path(&state.workspace_root, &path)
        .map_err(|e| Error::msg(format!("path resolution failed: {e}")))?;

    if let Some(gate) = security {
        let p = resolved.to_string_lossy().to_string();
        let pid = plugin_id.clone();
        let check = tokio::task::block_in_place(|| {
            state.runtime_handle.block_on(async move { gate.check_file_write(&pid, &p).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied write_file: {reason}")));
        }
    }

    tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            // Note: pass truncate=true to emulate standard write behavior
            let handle = state.vfs.open(&state.vfs_root_handle, &path, true, true).await?;
            let res = state.vfs.write(&handle, content.as_bytes()).await;
            let _ = state.vfs.close(&handle).await;
            res
        })
    }).map_err(|e| Error::msg(format!("write_file failed: {e}")))?;

    Ok(())
}