use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use extism::{CurrentPlugin, Error, UserData, Val};

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

    // We need to fetch the listener and the runtime handle out of the lock
    let (listener_arc, rt_handle) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        let listener = state
            .cli_socket_listener
            .clone()
            .ok_or_else(|| Error::msg("No CLI Socket Listener available in HostState"))?;

        (listener, state.runtime_handle.clone())
    };

    // Use the runtime handle to block on the async accept call
    let (stream, _addr) = rt_handle.block_on(async {
        let l = listener_arc.lock().await;
        l.accept().await
    })?;

    // Now re-acquire the lock to store the active stream and generate a handle ID
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let handle_id = state.active_streams.len() as u64 + 1;
    state.active_streams.insert(
        handle_id,
        std::sync::Arc::new(tokio::sync::Mutex::new(stream)),
    );

    // Return the handle ID as a string to the WASM plugin
    let mem = plugin.memory_new(handle_id.to_string())?;
    outputs[0] = plugin.memory_to_val(mem);

    Ok(())
}

pub(crate) fn astrid_net_read_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let handle_str = util::get_safe_string(plugin, &inputs[0], 1024)?;
    let handle_id: u64 = handle_str
        .parse()
        .map_err(|_| Error::msg("Invalid stream handle"))?;

    let ud = user_data.get()?;
    let (stream_arc, rt_handle) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        let stream = state
            .active_streams
            .get(&handle_id)
            .ok_or_else(|| Error::msg("Stream handle not found"))?
            .clone();
        (stream, state.runtime_handle.clone())
    };

    // We don't want to block the thread *forever* if there is no data,
    // otherwise the WASM execution will hang completely. So we need a timeout or a try_read,
    // but the `accept()` loop in the capsule expects blocking `read()`. We will do a short timeout
    // or rely on the capsule's timeout logic if they implement it.
    // For now, let's just do a blocking read into a buffer, but timeout after 50ms so we don't
    // lock the WASM engine if the CLI goes idle.
    use tokio::io::AsyncReadExt;

    let result = rt_handle.block_on(async {
        let mut stream = stream_arc.lock().await;
        let mut len_buf = [0u8; 4];

        // Wait for exactly 4 bytes (the length prefix used by the IPC protocol)
        if tokio::time::timeout(
            std::time::Duration::from_millis(50),
            stream.read_exact(&mut len_buf),
        )
        .await
        .is_err()
        {
            return Ok(Vec::new()); // Timeout, return empty
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 50 * 1024 * 1024 {
            return Err(Error::msg("Payload too large"));
        }

        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await?;

        Ok(payload)
    })?;

    if result.is_empty() {
        let mem = plugin.memory_new("")?;
        outputs[0] = plugin.memory_to_val(mem);
    } else {
        let mem = plugin.memory_new(&result)?;
        outputs[0] = plugin.memory_to_val(mem);
    }

    Ok(())
}

pub(crate) fn astrid_net_write_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let handle_str = util::get_safe_string(plugin, &inputs[0], 1024)?;
    let handle_id: u64 = handle_str
        .parse()
        .map_err(|_| Error::msg("Invalid stream handle"))?;
    let data = util::get_safe_bytes(plugin, &inputs[1], 10 * 1024 * 1024)?;

    let ud = user_data.get()?;
    let (stream_arc, rt_handle) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        let stream = state
            .active_streams
            .get(&handle_id)
            .ok_or_else(|| Error::msg("Stream handle not found"))?
            .clone();
        (stream, state.runtime_handle.clone())
    };

    use tokio::io::AsyncWriteExt;
    rt_handle.block_on(async {
        let mut stream = stream_arc.lock().await;
        // In the CLI architecture, we expect length-prefixed writes back to the client as well
        let len = data.len() as u32;
        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;
        Ok::<(), std::io::Error>(())
    })?;

    Ok(())
}
