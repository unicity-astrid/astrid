use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_events::AstridEvent;
use astrid_events::EventMetadata;
use astrid_events::ipc::{IpcMessage, IpcPayload};
use extism::{CurrentPlugin, Error, UserData, Val};

#[expect(clippy::needless_pass_by_value)]
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
    let topic =
        String::from_utf8(topic_bytes).map_err(|_| Error::msg("Topic is not valid UTF-8"))?;

    // Reject malformed topic structure before any matching or routing.
    if !crate::dispatcher::has_valid_segments(&topic) {
        return Err(Error::msg(
            "Topic contains empty segments (consecutive dots, leading/trailing dots, or is empty)",
        ));
    }

    if topic.split('.').count() > 8 {
        return Err(Error::msg("Topic exceeds maximum allowed segments (8)"));
    }

    // Enforce IPC topic publishing restrictions from Capsule.toml.
    // Fail-closed: capsules without ipc_publish declarations cannot publish.
    // Protected topics (kernel.*) require explicit declaration even if
    // a capsule has other patterns — defense-in-depth against privilege escalation.
    if state.ipc_publish_patterns.is_empty() {
        return Err(Error::msg(format!(
            "Capsule '{}' has no ipc_publish declarations — publishing is denied. \
             Add ipc_publish patterns to Capsule.toml [capabilities]",
            state.capsule_id
        )));
    }

    if !state
        .ipc_publish_patterns
        .iter()
        .any(|pattern| crate::dispatcher::topic_matches(&topic, pattern))
    {
        return Err(Error::msg(format!(
            "Capsule '{}' is not allowed to publish to topic '{topic}' — \
             declared ipc_publish patterns: {:?}",
            state.capsule_id, state.ipc_publish_patterns
        )));
    }

    let payload_bytes = util::get_safe_bytes(plugin, &inputs[1], util::MAX_GUEST_PAYLOAD_LEN)?;

    // Parse as raw JSON Value, then try to deserialize as a known IpcPayload variant.
    // If the payload matches any standard variant (RawJson, UserInput, OnboardingRequired, etc.),
    // use it directly. Otherwise fall back to Custom.
    let payload = match serde_json::from_slice::<serde_json::Value>(&payload_bytes) {
        Ok(data) => serde_json::from_value::<IpcPayload>(data.clone())
            .unwrap_or(IpcPayload::Custom { data }),
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

#[expect(clippy::needless_pass_by_value)]
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
    let topic_pattern = String::from_utf8(topic_pattern_bytes)
        .map_err(|_| Error::msg("Topic pattern is not valid UTF-8"))?;

    // Reject malformed subscription pattern structure before registration.
    if !crate::dispatcher::has_valid_segments(&topic_pattern) {
        return Err(Error::msg(
            "Topic pattern contains empty segments (consecutive dots, leading/trailing dots, or is empty)",
        ));
    }

    // EventReceiver::matches only supports trailing-suffix wildcards (e.g. `foo.bar.*`)
    // and exact matches. Mid-segment wildcards like `a.*.b` would silently never fire.
    // Reject them upfront with a clear error.
    {
        let mut segments = topic_pattern.split('.');
        // Use `position` (not `any`) to advance the iterator past the wildcard,
        // then check if there are trailing segments after it.
        #[expect(clippy::search_is_some)]
        if segments.position(|s| s == "*").is_some() && segments.next().is_some() {
            return Err(Error::msg(
                "Wildcard `*` is only supported as the last segment (e.g. `foo.bar.*`). \
                 Mid-segment wildcards like `a.*.b` are not supported by the event bus.",
            ));
        }
    }

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // Subscriptions are unprefixed. Capsules subscribe to system topics
    // directly (e.g., `agent.response`). Provenance is tracked via
    // `IpcMessage::source_id`, not topic namespacing.

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

#[expect(clippy::needless_pass_by_value)]
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
    let handle_id_str = String::from_utf8(handle_id_bytes)
        .map_err(|e| Error::msg(format!("Subscription handle is not valid UTF-8: {e}")))?;
    let handle_id: u64 = handle_id_str
        .parse()
        .map_err(|e| Error::msg(format!("Invalid subscription handle format: {e}")))?;

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
            let msg_len = serde_json::to_string(&message.payload)
                .map(|s| s.len())
                .unwrap_or(util::MAX_GUEST_PAYLOAD_LEN as usize);
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

    let lagged = receiver.drain_lagged();

    let result_obj = serde_json::json!({
        "messages": messages,
        "dropped": dropped,
        "lagged": lagged
    });

    let json = serde_json::to_string(&result_obj)
        .map_err(|e| Error::msg(format!("failed to serialize IPC messages: {e}")))?;

    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

/// Maximum timeout for blocking IPC receive (60 seconds).
const MAX_RECV_TIMEOUT_MS: u64 = 60_000;

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_ipc_recv_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let handle_id_bytes = util::get_safe_bytes(plugin, &inputs[0], 32)?;
    let handle_id_str = String::from_utf8(handle_id_bytes)
        .map_err(|e| Error::msg(format!("Subscription handle is not valid UTF-8: {e}")))?;
    let handle_id: u64 = handle_id_str
        .parse()
        .map_err(|e| Error::msg(format!("Invalid subscription handle format: {e}")))?;

    let timeout_bytes = util::get_safe_bytes(plugin, &inputs[1], 32)?;
    let timeout_str = String::from_utf8(timeout_bytes)
        .map_err(|e| Error::msg(format!("Timeout is not valid UTF-8: {e}")))?;
    let timeout_ms: u64 = timeout_str
        .parse()
        .map_err(|e| Error::msg(format!("Invalid timeout format: {e}")))?;
    let timeout_ms = timeout_ms.min(MAX_RECV_TIMEOUT_MS);

    let ud = user_data.get()?;

    // Temporarily remove the receiver from the map so we can drop the lock
    // before blocking. WASM is single-threaded so no concurrent access is possible.
    let (mut receiver, runtime_handle) = {
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        let receiver = state
            .subscriptions
            .remove(&handle_id)
            .ok_or_else(|| Error::msg("Subscription handle not found"))?;
        let runtime_handle = state.runtime_handle.clone();
        (receiver, runtime_handle)
    };

    // Block the WASM thread until a message arrives or timeout expires.
    // The WASM engine runs inside block_in_place, so blocking here is safe.
    let event = runtime_handle.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            receiver.recv(),
        )
        .await
    });

    // After the blocking wake, drain any additional messages that arrived
    // during the wait. This avoids N round-trips for N messages arriving
    // in a burst — the guest gets all of them in one call.
    let mut messages = Vec::new();
    let mut payload_bytes: usize = 0;
    let mut dropped: u64 = 0;

    if let Ok(Some(event)) = event
        && let AstridEvent::Ipc { message, .. } = &*event
    {
        let msg_len = serde_json::to_string(&message.payload)
            .map(|s| s.len())
            .unwrap_or(util::MAX_GUEST_PAYLOAD_LEN as usize);
        messages.push(message.clone());
        payload_bytes += msg_len;
    }

    // Drain remaining available messages (non-blocking).
    while let Some(event) = receiver.try_recv() {
        if let AstridEvent::Ipc { message, .. } = &*event {
            let msg_len = serde_json::to_string(&message.payload)
                .map(|s| s.len())
                .unwrap_or(util::MAX_GUEST_PAYLOAD_LEN as usize);
            if payload_bytes + msg_len > util::MAX_GUEST_PAYLOAD_LEN as usize {
                dropped += 1;
                break;
            }
            messages.push(message.clone());
            payload_bytes += msg_len;
        }
    }

    let lagged = receiver.drain_lagged();

    // Re-insert the receiver after draining
    {
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        state.subscriptions.insert(handle_id, receiver);
    }

    let result_obj = serde_json::json!({
        "messages": messages,
        "dropped": dropped,
        "lagged": lagged
    });

    let json = serde_json::to_string(&result_obj)
        .map_err(|e| Error::msg(format!("failed to serialize IPC messages: {e}")))?;

    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
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
    let handle_id_str = String::from_utf8(handle_id_bytes)
        .map_err(|e| Error::msg(format!("Subscription handle is not valid UTF-8: {e}")))?;
    let handle_id: u64 = handle_id_str
        .parse()
        .map_err(|e| Error::msg(format!("Invalid subscription handle format: {e}")))?;

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    if state.subscriptions.remove(&handle_id).is_none() {
        return Err(Error::msg("Subscription handle not found"));
    }

    Ok(())
}
