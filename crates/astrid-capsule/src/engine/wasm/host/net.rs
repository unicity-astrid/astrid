use extism::{CurrentPlugin, Error, UserData, Val};
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

// Stub implementation for now, will map to true UnixSockets soon!
pub(crate) fn astrid_net_bind_unix_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    let _path = util::get_safe_string(plugin, &inputs[0], 1024)?;
    // To support multiple sockets across multiple capsules, we'd need a registry in HostState.
    // For now, we return a mock handle.
    let mem = plugin.memory_new("mock_listener_id")?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

pub(crate) fn astrid_net_accept_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    let _handle = util::get_safe_string(plugin, &inputs[0], 1024)?;
    let mem = plugin.memory_new("mock_stream_id")?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
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