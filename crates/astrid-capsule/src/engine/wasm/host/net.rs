use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use extism::{CurrentPlugin, Error, UserData, Val};

/// Gate `net_bind` capability once at bind time (session-scoped).
///
/// The kernel pre-binds the socket and provides it via `HostState`. This
/// function enforces the security gate before the capsule can use the
/// listener — subsequent `accept()` calls do not re-check.
pub(crate) fn astrid_net_bind_unix_impl(
    _: &mut CurrentPlugin,
    _: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // Security gate: only capsules with net_bind capability may bind sockets.
    if let Some(ref gate) = state.security {
        let capsule_id = state.capsule_id.as_str().to_owned();
        let gate = gate.clone();
        let handle = state.runtime_handle.clone();
        tokio::task::block_in_place(|| {
            handle.block_on(async move { gate.check_net_bind(&capsule_id).await })
        })
        .map_err(|e| Error::msg(format!("security denied net_bind: {e}")))?;
    }

    // Return a dummy handle, since the socket is pre-bound.
    outputs[0] = Val::I64(1);
    Ok(())
}

pub(crate) fn astrid_net_accept_impl(
    plugin: &mut CurrentPlugin,
    _: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let ud = user_data.get()?;

    // We need to fetch the listener and the runtime handle out of the lock.
    // Security gate was already enforced at bind time (astrid_net_bind_unix_impl).
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

    // Use a monotonic counter to avoid handle ID reuse after stream removal.
    let handle_id = state.next_stream_id;
    state.next_stream_id = state
        .next_stream_id
        .checked_add(1)
        .ok_or_else(|| Error::msg("stream handle ID space exhausted"))?;
    debug_assert!(
        !state.active_streams.contains_key(&handle_id),
        "stream handle ID collision"
    );
    state.active_streams.insert(
        handle_id,
        std::sync::Arc::new(tokio::sync::Mutex::new(stream)),
    );

    // Notify the kernel that a new client connection was accepted so the
    // idle monitor can track active connections.
    let connected_msg = astrid_events::ipc::IpcMessage::new(
        "client.connected",
        astrid_events::ipc::IpcPayload::Connect,
        state.capsule_uuid,
    );
    let _ = state.event_bus.publish(astrid_events::AstridEvent::Ipc {
        metadata: astrid_events::EventMetadata::new("net_accept"),
        message: connected_msg,
    });

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

        // Wait for exactly 4 bytes (the length prefix used by the IPC protocol).
        // Distinguish between a genuine timeout (no data yet) and an I/O error
        // (peer disconnect, broken pipe) to avoid spin-looping on dead connections.
        match tokio::time::timeout(
            std::time::Duration::from_millis(50),
            stream.read_exact(&mut len_buf),
        )
        .await
        {
            Err(_) => return Ok(Vec::new()), // Genuine timeout, no data yet
            Ok(Err(e)) => return Err(Error::msg(format!("socket read error: {e}"))),
            Ok(Ok(_)) => {}, // Got the 4-byte length prefix
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 10 * 1024 * 1024 {
            return Err(Error::msg("Payload too large (max 10MB)"));
        }

        let mut payload = vec![0u8; len];
        // Timeout proportional to payload size: 5s base + 1s per MB.
        let timeout_ms = 5000 + (len as u64 / 1024);
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            stream.read_exact(&mut payload),
        )
        .await
        .map_err(|_| Error::msg("Payload read timed out"))?
        .map_err(|e| Error::msg(format!("socket payload read error: {e}")))?;

        Ok(payload)
    });

    // If the socket read failed (connection closed, broken pipe), publish a
    // client.disconnect event so the idle monitor is notified even if the
    // WASM proxy capsule doesn't explicitly forward the Disconnect message.
    if let Err(ref e) = result {
        let err_str = e.to_string();
        if (err_str.contains("socket read error") || err_str.contains("socket payload read error"))
            && let Ok(state) = ud.lock()
        {
            let msg = astrid_events::ipc::IpcMessage::new(
                "client.disconnect",
                astrid_events::ipc::IpcPayload::Disconnect {
                    reason: Some("socket_closed".to_string()),
                },
                state.capsule_uuid,
            );
            let _ = state.event_bus.publish(astrid_events::AstridEvent::Ipc {
                metadata: astrid_events::EventMetadata::new("net_read"),
                message: msg,
            });
        }
    }

    let result = result?;

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
    _: &mut [Val],
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
        let len = u32::try_from(data.len())
            .map_err(|_| std::io::Error::other("write payload too large for length prefix"))?;
        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;
        Ok::<(), std::io::Error>(())
    })?;

    Ok(())
}
