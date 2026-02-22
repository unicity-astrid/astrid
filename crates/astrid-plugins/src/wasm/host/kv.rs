use crate::wasm::host::util;
use crate::wasm::host_state::HostState;
use extism::{CurrentPlugin, Error, UserData, Val};

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kv_get_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = util::get_safe_string(plugin, &inputs[0], util::MAX_KEY_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let kv = state.kv.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let result = handle.block_on(async { kv.get(&key).await });

    let value = match result {
        Ok(Some(bytes)) => {
            if bytes.len() as u64 > util::MAX_GUEST_PAYLOAD_LEN {
                return Err(Error::msg(
                    "KV value exceeds maximum allowed guest payload limit",
                ));
            }
            String::from_utf8_lossy(&bytes).into_owned()
        },
        Ok(None) => String::new(),
        Err(e) => return Err(Error::msg(format!("kv_get failed: {e}"))),
    };

    let mem = plugin.memory_new(&value)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kv_set_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = util::get_safe_string(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let value: String = util::get_safe_string(plugin, &inputs[1], util::MAX_GUEST_PAYLOAD_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let kv = state.kv.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let result = handle.block_on(async { kv.set(&key, value.into_bytes()).await });

    match result {
        Ok(()) => Ok(()),
        Err(e) => Err(Error::msg(format!("kv_set failed: {e}"))),
    }
}
