use crate::wasm::host_state::HostState;
use astrid_events::AstridEvent;
use astrid_events::EventMetadata;
use astrid_events::ipc::{IpcMessage, IpcPayload};
use extism::{CurrentPlugin, Error, UserData, Val};

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_ipc_publish_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    // Prevent IPC topic abuse
    let topic_ptr = inputs[0].unwrap_i64();
    let topic_len = plugin.memory_length(topic_ptr.cast_unsigned())?;
    if topic_len > 256 {
        return Err(Error::msg("Topic exceeds maximum allowed length (256 bytes)"));
    }

    let payload_ptr = inputs[1].unwrap_i64();
    let payload_len = plugin.memory_length(payload_ptr.cast_unsigned())?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // Check rate limit and quotas using the length *before* allocating the memory
    state
        .ipc_limiter
        .check_quota(state.plugin_uuid, payload_len.try_into().unwrap_or(usize::MAX))
        .map_err(|err| Error::msg(format!("IPC rate limit exceeded: {err}")))?;

    let topic: String = plugin.memory_get_val(&inputs[0])?;
    let payload_bytes: Vec<u8> = plugin.memory_get_val(&inputs[1])?;

    // Attempt to deserialize payload. If it fails, wrap it in Custom.
    let payload = if let Ok(p) = serde_json::from_slice::<IpcPayload>(&payload_bytes) {
        p
    } else {
        // Fallback to custom unstructured data
        let data = serde_json::from_slice::<serde_json::Value>(&payload_bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&payload_bytes).into_owned()));
        IpcPayload::Custom { data }
    };

    let message = IpcMessage::new(topic, payload, state.plugin_uuid);

    let event = AstridEvent::Ipc {
        metadata: EventMetadata::new("wasm_guest")
            .with_session_id(state.plugin_uuid),
        message,
    };

    // Publish to the event bus
    state.event_bus.publish(event);

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_ipc_subscribe_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let topic_pattern: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    if state.subscriptions.len() >= 128 {
        return Err(Error::msg("Subscription limit reached (128 max per plugin)"));
    }

    // In a full implementation, we would register this receiver in HostState
    // and provide an `astrid_ipc_poll` function to read from it.
    let receiver = state.event_bus.subscribe_topic(topic_pattern);
    
    // For Phase 1, we store the receiver in HostState to keep the subscription alive.
    // We use a simple counter for handle IDs.
    let handle_id = state.next_subscription_id;
    state.next_subscription_id = state.next_subscription_id.wrapping_add(1);
    state.subscriptions.insert(handle_id, receiver);

    let handle_str = handle_id.to_string();
    let mem = plugin.memory_new(&handle_str)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}
