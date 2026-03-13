use astrid_core::session_token::{
    HandshakeRequest, HandshakeResponse, PROTOCOL_VERSION, SessionToken,
};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use extism::{CurrentPlugin, Error, UserData, Val};

/// Gate `net_bind` capability once at bind time (session-scoped).
///
/// The kernel pre-binds the socket and provides it via `HostState`. This
/// function enforces the security gate before the capsule can use the
/// listener - subsequent `accept()` calls do not re-check.
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
        let semaphore = state.host_semaphore.clone();
        util::bounded_block_on(&handle, &semaphore, async move {
            gate.check_net_bind(&capsule_id).await
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

    // We need to fetch the listener, runtime handle, cancel token, session
    // token, and host semaphore out of the lock. Security gate was already
    // enforced at bind time.
    let (listener_arc, rt_handle, cancel_token, session_token, host_semaphore) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        let listener = state
            .cli_socket_listener
            .clone()
            .ok_or_else(|| Error::msg("No CLI Socket Listener available in HostState"))?;

        (
            listener,
            state.runtime_handle.clone(),
            state.cancel_token.clone(),
            state.session_token.clone(),
            state.host_semaphore.clone(),
        )
    };

    // Accept + authenticate loop. Authentication failures (wrong UID, bad
    // token) retry accept immediately so a malicious client cannot gate
    // legitimate connections behind the WASM-side 100ms backoff. Only real
    // listener errors (EMFILE, EBADF) or cancellation propagate to the WASM
    // capsule.
    let stream = loop {
        // Respects cancellation so unload doesn't hang waiting for a connection.
        // The listener Mutex is held for the duration of accept(). This is
        // correct because this is a single-client design - only one WASM
        // capsule thread calls accept(), and no other code path contends.
        //
        // Cancel safety: accept() is cancel-safe - dropping mid-accept is
        // harmless, the listener remains valid for subsequent calls.
        let accept_result =
            util::bounded_block_on_cancellable(&rt_handle, &host_semaphore, &cancel_token, async {
                let l = listener_arc.lock().await;
                l.accept().await
            });
        let (stream, _addr) = match accept_result {
            Some(result) => result?,
            None => return Err(Error::msg("capsule unloading")),
        };

        // Peer credential verification - reject connections from different UIDs.
        // Runs before token handshake to prevent cross-UID DoS via the 5s timeout.
        #[cfg(unix)]
        if let Err(reason) = verify_peer_credentials(&stream) {
            tracing::warn!(
                security_event = true,
                reason = %reason,
                "Rejected socket connection: peer credential check failed"
            );
            drop(stream);
            continue;
        }

        // Authenticate the connection via session token handshake.
        // The stream is a local variable (not behind any lock), so this
        // cannot deadlock. The 5s timeout prevents a malicious client from
        // holding the accept loop hostage.
        let mut stream = stream;
        if let Some(ref token) = session_token {
            let handshake_result = util::bounded_block_on_cancellable(
                &rt_handle,
                &host_semaphore,
                &cancel_token,
                validate_handshake(&mut stream, token),
            );
            match handshake_result {
                None => return Err(Error::msg("capsule unloading")),
                Some(Ok(())) => break stream,
                Some(Err(reason)) => {
                    tracing::warn!(
                        security_event = true,
                        reason = %reason,
                        "Rejected socket connection: handshake failed"
                    );
                    drop(stream);
                    continue;
                },
            }
        } else {
            // No session token configured (test/legacy mode) - accept without auth.
            break stream;
        }
    };

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
        "client.v1.connected",
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
    let (stream_arc, rt_handle, cancel_token, host_semaphore) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        let stream = state
            .active_streams
            .get(&handle_id)
            .ok_or_else(|| Error::msg("Stream handle not found"))?
            .clone();
        (
            stream,
            state.runtime_handle.clone(),
            state.cancel_token.clone(),
            state.host_semaphore.clone(),
        )
    };

    // Cancel safety: read_exact is not cancel-safe, so cancellation mid-read
    // may leave a partial frame on the socket. This is acceptable because the
    // capsule is unloading - the socket will be closed by Drop on
    // active_streams and the client will see a hard EOF / connection reset.
    use tokio::io::AsyncReadExt;

    let result =
        util::bounded_block_on_cancellable(&rt_handle, &host_semaphore, &cancel_token, async {
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
    // Cancellation returns empty bytes - same as the prior timeout/cancel
    // behaviour, signalling "no data yet" to the WASM capsule.
    let result = match result {
        Some(r) => r,
        None => Ok(Vec::new()),
    };

    // If the socket read failed (connection closed, broken pipe), publish a
    // client.disconnect event so the idle monitor is notified even if the
    // WASM proxy capsule doesn't explicitly forward the Disconnect message.
    if let Err(ref e) = result {
        let err_str = e.to_string();
        if (err_str.contains("socket read error") || err_str.contains("socket payload read error"))
            && let Ok(state) = ud.lock()
        {
            let msg = astrid_events::ipc::IpcMessage::new(
                "client.v1.disconnect",
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
    let (stream_arc, rt_handle, host_semaphore, cancel_token) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        let stream = state
            .active_streams
            .get(&handle_id)
            .ok_or_else(|| Error::msg("Stream handle not found"))?
            .clone();
        (
            stream,
            state.runtime_handle.clone(),
            state.host_semaphore.clone(),
            state.cancel_token.clone(),
        )
    };

    use tokio::io::AsyncWriteExt;
    // Cancel safety: write_all is not cancel-safe, so cancellation mid-write
    // may leave a partial frame on the socket. This is acceptable because the
    // capsule is unloading - the socket will be closed by Drop on
    // active_streams and the client will see a hard EOF / connection reset.
    let result =
        util::bounded_block_on_cancellable(&rt_handle, &host_semaphore, &cancel_token, async {
            let mut stream = stream_arc.lock().await;
            // In the CLI architecture, we expect length-prefixed writes back to the client as well
            let len = u32::try_from(data.len())
                .map_err(|_| std::io::Error::other("write payload too large for length prefix"))?;
            stream.write_all(&len.to_be_bytes()).await?;
            stream.write_all(&data).await?;
            stream.flush().await?;
            Ok::<(), std::io::Error>(())
        });
    match result {
        Some(inner) => inner?,
        None => return Err(Error::msg("capsule unloading")),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Handshake helpers
// ---------------------------------------------------------------------------

/// Timeout for individual handshake read/write operations (server-side).
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Maximum allowed size of a handshake request payload (bytes).
const MAX_HANDSHAKE_SIZE: usize = 4096;

/// Validate the client handshake: read the `HandshakeRequest`, verify the token
/// and protocol version, then send back a `HandshakeResponse`.
///
/// Returns `Ok(())` on success or `Err(reason)` with a human-readable rejection
/// reason.
async fn validate_handshake(
    stream: &mut tokio::net::UnixStream,
    expected_token: &SessionToken,
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;

    // 1. Read the handshake request (length-prefixed JSON, same wire format).
    let mut len_buf = [0u8; 4];
    tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut len_buf))
        .await
        .map_err(|_| "handshake timed out (5s)".to_string())?
        .map_err(|e| format!("handshake read error: {e}"))?;

    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_HANDSHAKE_SIZE {
        return Err(format!("handshake too large: {len} bytes"));
    }

    let mut payload = vec![0u8; len];
    tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut payload))
        .await
        .map_err(|_| "handshake payload timed out".to_string())?
        .map_err(|e| format!("handshake payload read error: {e}"))?;

    let request: HandshakeRequest =
        serde_json::from_slice(&payload).map_err(|e| format!("invalid handshake JSON: {e}"))?;

    // 2. Validate protocol version FIRST - this check reveals no information
    // about token validity. Checking version before token prevents an oracle
    // where a "protocol mismatch" response confirms the token was correct.
    if request.protocol_version != PROTOCOL_VERSION {
        let reason = format!(
            "Protocol version mismatch (client={}, server={}). \
             Restart the daemon with `astrid daemon restart`.",
            request.protocol_version, PROTOCOL_VERSION,
        );
        if let Err(e) =
            send_handshake_response_timed(stream, &HandshakeResponse::error(&reason)).await
        {
            tracing::warn!(error = %e, "Failed to send handshake error response for protocol mismatch");
        }
        return Err(reason);
    }

    // 3. Validate token (constant-time comparison).
    // Send a uniform error response on both malformed-hex and wrong-token
    // paths to prevent an oracle that distinguishes the two failure modes.
    let client_token = match SessionToken::from_hex(&request.token) {
        Ok(t) => t,
        Err(_) => {
            if let Err(e) = send_handshake_response_timed(
                stream,
                &HandshakeResponse::error("authentication failed"),
            )
            .await
            {
                tracing::warn!(error = %e, "Failed to send handshake error response");
            }
            return Err("invalid session token".to_string());
        },
    };

    if !expected_token.ct_eq(&client_token) {
        if let Err(e) = send_handshake_response_timed(
            stream,
            &HandshakeResponse::error("authentication failed"),
        )
        .await
        {
            tracing::warn!(error = %e, "Failed to send handshake error response");
        }
        return Err("invalid session token".to_string());
    }

    // 4. All checks passed - send success response.
    send_handshake_response_timed(stream, &HandshakeResponse::ok())
        .await
        .map_err(|e| format!("failed to send handshake response: {e}"))?;

    // Truncate client_version to prevent log injection from oversized values.
    // Use chars().take() to avoid panicking on multi-byte UTF-8 boundaries.
    let safe_version: String = request.client_version.chars().take(64).collect();
    tracing::info!(
        client_version = %safe_version,
        "Socket handshake succeeded"
    );
    Ok(())
}

/// Send a length-prefixed JSON handshake response with a 5s write timeout.
///
/// Wraps [`send_handshake_response`] with a timeout to prevent a stalled
/// client from holding the accept loop hostage during the response write.
async fn send_handshake_response_timed(
    stream: &mut tokio::net::UnixStream,
    response: &HandshakeResponse,
) -> Result<(), std::io::Error> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, send_handshake_response(stream, response))
        .await
        .map_err(|_| std::io::Error::other("handshake response write timed out (5s)"))?
}

/// Send a length-prefixed JSON handshake response.
async fn send_handshake_response(
    stream: &mut tokio::net::UnixStream,
    response: &HandshakeResponse,
) -> Result<(), std::io::Error> {
    use tokio::io::AsyncWriteExt;

    let bytes = serde_json::to_vec(response)
        .map_err(|e| std::io::Error::other(format!("serialize handshake response: {e}")))?;
    let len = u32::try_from(bytes.len())
        .map_err(|_| std::io::Error::other("handshake response too large"))?;

    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// Verify that the connecting process runs as the same UID as the daemon.
/// Returns `Err(reason)` if the UID does not match or credentials cannot
/// be retrieved.
#[cfg(unix)]
fn verify_peer_credentials(stream: &tokio::net::UnixStream) -> Result<(), String> {
    match stream.peer_cred() {
        Ok(cred) => {
            let peer_uid = cred.uid();
            let my_uid = nix::unistd::geteuid().as_raw();
            if peer_uid != my_uid {
                Err(format!(
                    "peer UID {peer_uid} does not match daemon UID {my_uid}"
                ))
            } else {
                Ok(())
            }
        },
        Err(e) => Err(format!("failed to check peer credentials: {e}")),
    }
}
