use extism::{CurrentPlugin, Error, UserData, Val};
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

// Note: `bind_unix` is ignored because the Kernel natively binds the socket and passes 
// it into the HostState. The WASM module just calls accept() directly.
pub(crate) fn astrid_net_bind_unix_impl(
    _plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    // Return a dummy handle, since the socket is pre-bound.
    outputs[0] = Val::I64(1);
    Ok(())
}

pub(crate) fn astrid_net_accept_impl(
    plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    if let Some(_listener_mutex) = &state.cli_socket_listener {
        // We cannot `await` inside a synchronous Extism host function.
        // This requires an async bridge or using `try_accept()`.
        // For Phase 8 we will just return a mock stream ID to get it compiling,
        // and we will rewrite the bridge to be fully async in the next PR.
        let mem = plugin.memory_new("mock_stream_id")?;
        outputs[0] = plugin.memory_to_val(mem);
        Ok(())
    } else {
        Err(Error::msg("No CLI Socket Listener available in HostState"))
    }
}

pub(crate) fn astrid_net_read_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    let _handle = util::get_safe_string(plugin, &inputs[0], 1024)?;
    let mem = plugin.memory_new("{}")?; // Empty JSON mock
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

pub(crate) fn astrid_net_write_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    let _handle = util::get_safe_string(plugin, &inputs[0], 1024)?;
    let _data = util::get_safe_bytes(plugin, &inputs[1], 10 * 1024 * 1024)?;
    Ok(())
}
