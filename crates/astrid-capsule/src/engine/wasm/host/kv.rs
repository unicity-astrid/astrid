use extism::{CurrentPlugin, Error, UserData, Val};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kv_get_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let key = String::from_utf8(key_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let result = tokio::task::block_in_place(|| {
        state
            .runtime_handle
            .block_on(async { state.kv.get(&key).await })
    })
    .map_err(|e| Error::msg(format!("kv_get failed: {e}")))?;

    let value_bytes = match result {
        Some(v) => serde_json::to_vec(&v).unwrap_or_default(),
        None => Vec::new(),
    };

    let mem = plugin.memory_new(&value_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kv_set_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let value_bytes: Vec<u8> =
        util::get_safe_bytes(plugin, &inputs[1], util::MAX_GUEST_PAYLOAD_LEN)?;

    let key = String::from_utf8(key_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    tokio::task::block_in_place(|| {
        state.runtime_handle.block_on(async {
            // KV storage takes Vec<u8> directly.
            state.kv.set(&key, value_bytes).await
        })
    })
    .map_err(|e| Error::msg(format!("kv_set failed: {e}")))?;

    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kv_delete_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let key = String::from_utf8(key_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    tokio::task::block_in_place(|| {
        state
            .runtime_handle
            .block_on(async { state.kv.delete(&key).await })
    })
    .map_err(|e| Error::msg(format!("kv_delete failed: {e}")))?;

    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kv_list_keys_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let prefix_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let prefix = String::from_utf8(prefix_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let keys = tokio::task::block_in_place(|| {
        state
            .runtime_handle
            .block_on(async { state.kv.list_keys_with_prefix(&prefix).await })
    })
    .map_err(|e| Error::msg(format!("kv_list_keys failed: {e}")))?;

    let result_bytes = serde_json::to_vec(&keys).unwrap_or_default();
    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kv_clear_prefix_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let prefix_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let prefix = String::from_utf8(prefix_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let count = tokio::task::block_in_place(|| {
        state
            .runtime_handle
            .block_on(async { state.kv.clear_prefix(&prefix).await })
    })
    .map_err(|e| Error::msg(format!("kv_clear_prefix failed: {e}")))?;

    let result_bytes = serde_json::to_vec(&count).unwrap_or_default();
    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}
