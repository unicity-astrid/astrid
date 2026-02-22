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
    let topic: String = plugin.memory_get_val(&inputs[0])?;
    let payload_bytes: Vec<u8> = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let plugin_uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, state.plugin_id.as_str().as_bytes());

    // Check rate limit and quotas
    state
        .ipc_limiter
        .check_quota(plugin_uuid, payload_bytes.len())
        .map_err(|err| Error::msg(format!("IPC rate limit exceeded: {err}")))?;

    // Attempt to deserialize payload. If it fails, wrap it in Custom.
    let payload = if let Ok(p) = serde_json::from_slice::<IpcPayload>(&payload_bytes) {
        p
    } else {
        // Fallback to custom unstructured data
        let data = serde_json::from_slice::<serde_json::Value>(&payload_bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&payload_bytes).into_owned()));
        IpcPayload::Custom { data }
    };

    let message = IpcMessage::new(topic, payload, plugin_uuid);

    let event = AstridEvent::Ipc {
        metadata: EventMetadata::new("wasm_guest")
            .with_session_id(plugin_uuid),
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
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // In a full implementation, we would register this receiver in HostState
    // and provide an `astrid_ipc_poll` function to read from it.
    let _receiver = state.event_bus.subscribe_topic(topic_pattern);
    
    // For Phase 1, we just return a dummy subscription handle (1).
    let handle_id = 1_u64;

    outputs[0] = Val::I64(handle_id.cast_signed());
    Ok(())
}
