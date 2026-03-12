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
    let (kv, runtime_handle, host_semaphore) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        (
            state.kv.clone(),
            state.runtime_handle.clone(),
            state.host_semaphore.clone(),
        )
    };

    let result = util::bounded_block_on(&runtime_handle, &host_semaphore, async {
        kv.get(&key).await
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
    let (kv, runtime_handle, host_semaphore) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        (
            state.kv.clone(),
            state.runtime_handle.clone(),
            state.host_semaphore.clone(),
        )
    };

    util::bounded_block_on(&runtime_handle, &host_semaphore, async {
        // KV storage takes Vec<u8> directly.
        kv.set(&key, value_bytes).await
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
    let (kv, runtime_handle, host_semaphore) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        (
            state.kv.clone(),
            state.runtime_handle.clone(),
            state.host_semaphore.clone(),
        )
    };

    util::bounded_block_on(&runtime_handle, &host_semaphore, async {
        kv.delete(&key).await
    })
    .map_err(|e| Error::msg(format!("kv_delete failed: {e}")))?;

    Ok(())
}
