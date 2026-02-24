use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
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
        return Err(Error::msg(
            "Topic exceeds maximum allowed length (256 bytes)",
        ));
    }

    let payload_ptr = inputs[1].unwrap_i64();
    let payload_len = plugin.memory_length(payload_ptr.cast_unsigned())?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // Check rate limit and quotas using the length *before* allocating the memory
    if let Err(e) = state.ipc_limiter.check_quota(
        state.capsule_uuid,
        payload_len.try_into().unwrap_or(usize::MAX),
    ) {
        return Err(Error::msg(e.to_string()));
    }

    let topic_bytes = util::get_safe_bytes(plugin, &inputs[0], 256)?;
    let mut topic = String::from_utf8(topic_bytes).unwrap_or_default();

    // Auto-prefix with capsule ID to securely namespace topics
    let prefix = format!("{}.", state.capsule_id.as_str());
    if !topic.starts_with(&prefix) {
        topic = format!("{prefix}{topic}");
    }

    if topic.split('.').count() > 8 {
        return Err(Error::msg("Topic exceeds maximum allowed segments (8)"));
    }

    let payload_bytes = util::get_safe_bytes(plugin, &inputs[1], util::MAX_GUEST_PAYLOAD_LEN)?;

    // Parse as raw JSON Value first
    let payload = match serde_json::from_slice::<serde_json::Value>(&payload_bytes) {
        Ok(data) => {
            // Check if it declares a standard payload type
            if let Some(type_str) = data.get("type").and_then(|t| t.as_str()) {
                match type_str {
                    "user_input" | "agent_response" | "approval_required" | "custom" => {
                        // Try to strictly parse as IpcPayload to catch schema typos
                        serde_json::from_value::<IpcPayload>(data)
                            .map_err(|e| Error::msg(format!("Invalid standard IPC schema: {e}")))?
                    },
                    _ => IpcPayload::Custom { data },
                }
            } else {
                // Fallback for missing type
                IpcPayload::Custom { data }
            }
        },
        Err(_) => return Err(Error::msg("IPC payload is not valid JSON")),
    };

    let message = IpcMessage::new(topic, payload, state.capsule_uuid);

    let event = AstridEvent::Ipc {
        metadata: EventMetadata::new("wasm_guest").with_session_id(state.capsule_uuid),
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
    let topic_pattern_ptr = inputs[0].unwrap_i64();
    let topic_pattern_len = plugin.memory_length(topic_pattern_ptr.cast_unsigned())?;
    if topic_pattern_len > 256 {
        return Err(Error::msg(
            "Topic pattern exceeds maximum allowed length (256 bytes)",
        ));
    }

    let topic_pattern_bytes = util::get_safe_bytes(plugin, &inputs[0], 256)?;
    let mut topic_pattern = String::from_utf8(topic_pattern_bytes).unwrap_or_default();

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // Auto-prefix with capsule ID to securely namespace subscriptions
    let prefix = format!("{}.", state.capsule_id.as_str());
    if !topic_pattern.starts_with(&prefix) {
        topic_pattern = format!("{prefix}{topic_pattern}");
    }

    if topic_pattern.split('.').count() > 8 {
        return Err(Error::msg(
            "Topic pattern exceeds maximum allowed segments (8)",
        ));
    }

    if state.subscriptions.len() >= 128 {
        return Err(Error::msg(
            "Subscription limit reached (128 max per plugin)",
        ));
    }

    let receiver = state.event_bus.subscribe_topic(topic_pattern);

    let handle_id = state.next_subscription_id;
    if state.subscriptions.contains_key(&handle_id) {
        return Err(Error::msg(
            "Subscription handle ID collision due to wraparound",
        ));
    }

    let handle_str = handle_id.to_string();
    let mem = plugin.memory_new(&handle_str)?;

    state.next_subscription_id = state.next_subscription_id.wrapping_add(1);
    state.subscriptions.insert(handle_id, receiver);

    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_ipc_poll_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let handle_ptr = inputs[0].unwrap_i64();
    let handle_len = plugin.memory_length(handle_ptr.cast_unsigned())?;
    if handle_len > 32 {
        return Err(Error::msg(
            "Subscription handle exceeds maximum allowed length",
        ));
    }

    let handle_id_bytes = util::get_safe_bytes(plugin, &inputs[0], 32)?;
    let handle_id_str = String::from_utf8(handle_id_bytes).unwrap_or_default();
    let handle_id: u64 = handle_id_str
        .parse()
        .map_err(|_| Error::msg("Invalid subscription handle format"))?;

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let receiver = state
        .subscriptions
        .get_mut(&handle_id)
        .ok_or_else(|| Error::msg("Subscription handle not found"))?;

    let mut messages = Vec::new();
    let mut payload_bytes = 0;

    let mut dropped = 0;

    // Non-blocking poll - drain until buffer full or no more messages
    while let Some(event) = receiver.try_recv() {
        if let AstridEvent::Ipc { message, .. } = &*event {
            let msg_len = serde_json::to_string(&message.payload).map(|s| s.len()).unwrap_or(util::MAX_GUEST_PAYLOAD_LEN as usize);
            if payload_bytes + msg_len > util::MAX_GUEST_PAYLOAD_LEN as usize {
                // Buffer full, drop the current message and leave the rest in the channel.
                // NOTE: The message that triggered this overflow is permanently consumed from 
                // the broadcast receiver and lost. The `dropped` counter signals this loss to the guest.
                dropped += 1;
                break;
            }
            messages.push(message.clone());
            payload_bytes += msg_len;
        }
    }

    let result_obj = serde_json::json!({
        "messages": messages,
        "dropped": dropped
    });

    let json = serde_json::to_string(&result_obj)
        .map_err(|e| Error::msg(format!("failed to serialize IPC messages: {e}")))?;

    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_ipc_unsubscribe_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let handle_ptr = inputs[0].unwrap_i64();
    let handle_len = plugin.memory_length(handle_ptr.cast_unsigned())?;
    if handle_len > 32 {
        return Err(Error::msg(
            "Subscription handle exceeds maximum allowed length",
        ));
    }

    let handle_id_bytes = util::get_safe_bytes(plugin, &inputs[0], 32)?;
    let handle_id_str = String::from_utf8(handle_id_bytes).unwrap_or_default();
    let handle_id: u64 = handle_id_str
        .parse()
        .map_err(|_| Error::msg("Invalid subscription handle format"))?;

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    if state.subscriptions.remove(&handle_id).is_none() {
        return Err(Error::msg("Subscription handle not found"));
    }

    Ok(())
}
