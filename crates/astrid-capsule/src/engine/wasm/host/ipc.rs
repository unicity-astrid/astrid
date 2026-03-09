use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_events::AstridEvent;
use astrid_events::EventMetadata;
use astrid_events::EventReceiver;
use astrid_events::ipc::{IpcMessage, IpcPayload};
use extism::{CurrentPlugin, Error, UserData, Val};

// ── Extracted testable core ─────────────────────────────────────────

/// Result of draining IPC messages from an `EventReceiver`.
#[cfg_attr(test, derive(Debug))]
pub(crate) struct DrainResult {
    pub messages: Vec<IpcMessage>,
    pub dropped: u64,
    pub lagged: u64,
}

/// Drain all available IPC messages from a receiver (non-blocking).
///
/// Collects messages until the buffer exceeds `max_payload_bytes` or no
/// more messages are available. Returns the collected messages, a count
/// of messages dropped due to buffer overflow, and the cumulative lag.
pub(crate) fn drain_receiver(
    receiver: &mut EventReceiver,
    max_payload_bytes: usize,
) -> DrainResult {
    let mut messages = Vec::new();
    let mut payload_bytes: usize = 0;
    let mut dropped: u64 = 0;

    while let Some(event) = receiver.try_recv() {
        if let AstridEvent::Ipc { message, .. } = &*event {
            let msg_len = serde_json::to_string(&message.payload)
                .map(|s| s.len())
                .unwrap_or(max_payload_bytes);
            if payload_bytes + msg_len > max_payload_bytes {
                dropped += 1;
                break;
            }
            messages.push(message.clone());
            payload_bytes += msg_len;
        }
    }

    let lagged = receiver.drain_lagged();

    DrainResult {
        messages,
        dropped,
        lagged,
    }
}

/// Serialize a drain result into the standard IPC poll/recv JSON envelope.
pub(crate) fn serialize_envelope(result: &DrainResult) -> Result<String, Error> {
    let obj = serde_json::json!({
        "messages": result.messages,
        "dropped": result.dropped,
        "lagged": result.lagged
    });
    serde_json::to_string(&obj)
        .map_err(|e| Error::msg(format!("failed to serialize IPC messages: {e}")))
}

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

    let drain = drain_receiver(receiver, util::MAX_GUEST_PAYLOAD_LEN as usize);
    let json = serialize_envelope(&drain)?;

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

    // Collect the blocking-wake message (if any) plus drain remaining.
    let mut drain = drain_receiver(&mut receiver, util::MAX_GUEST_PAYLOAD_LEN as usize);

    // Prepend the message that woke us (it was consumed by recv, not try_recv).
    if let Ok(Some(event)) = event
        && let AstridEvent::Ipc { message, .. } = &*event
    {
        drain.messages.insert(0, message.clone());
    }

    // Re-insert the receiver after draining
    {
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        state.subscriptions.insert(handle_id, receiver);
    }

    let json = serialize_envelope(&drain)?;

    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrid_events::EventBus;
    use astrid_events::ipc::IpcPayload;

    /// Publish N IPC messages to a bus and return a receiver subscribed to them.
    fn publish_ipc_messages(bus: &EventBus, topic: &str, count: usize) {
        for i in 0..count {
            let msg = IpcMessage::new(
                topic,
                IpcPayload::Custom {
                    data: serde_json::json!({"i": i}),
                },
                uuid::Uuid::new_v4(),
            );
            bus.publish(AstridEvent::Ipc {
                metadata: EventMetadata::new("test"),
                message: msg,
            });
        }
    }

    #[test]
    fn drain_receiver_collects_all_available_messages() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe_topic("test.topic");

        publish_ipc_messages(&bus, "test.topic", 5);

        let result = drain_receiver(&mut receiver, 10 * 1024 * 1024);
        assert_eq!(result.messages.len(), 5);
        assert_eq!(result.dropped, 0);
        assert_eq!(result.lagged, 0);
    }

    #[test]
    fn drain_receiver_returns_empty_when_no_messages() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe_topic("test.topic");

        let result = drain_receiver(&mut receiver, 10 * 1024 * 1024);
        assert!(result.messages.is_empty());
        assert_eq!(result.dropped, 0);
        assert_eq!(result.lagged, 0);
    }

    #[test]
    fn drain_receiver_drops_on_buffer_overflow() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe_topic("test.topic");

        // Publish messages — each has a small JSON payload.
        publish_ipc_messages(&bus, "test.topic", 10);

        // Use a tiny buffer limit so the drain hits the overflow.
        // Each message payload `{"i":0}` serializes to ~20+ bytes as IpcPayload.
        let result = drain_receiver(&mut receiver, 50);
        assert!(
            result.messages.len() < 10,
            "should not have drained all 10 with 50-byte limit, got {}",
            result.messages.len()
        );
        assert_eq!(
            result.dropped, 1,
            "one message should be dropped on overflow"
        );
    }

    #[test]
    fn drain_receiver_surfaces_lag_from_broadcast_overflow() {
        // Tiny channel capacity to force lag.
        let bus = EventBus::with_capacity(2);
        let mut receiver = bus.subscribe_topic("test.topic");

        // Publish 5 messages into a capacity-2 channel — the receiver will lag.
        publish_ipc_messages(&bus, "test.topic", 5);

        let result = drain_receiver(&mut receiver, 10 * 1024 * 1024);
        assert!(
            result.lagged > 0,
            "expected lag from broadcast overflow, got 0"
        );
        // Should still receive the messages that weren't lost.
        assert!(
            !result.messages.is_empty(),
            "should still get some messages after lag"
        );
    }

    #[test]
    fn serialize_envelope_produces_valid_json_with_all_fields() {
        let msg = IpcMessage::new(
            "test.topic",
            IpcPayload::Custom {
                data: serde_json::json!({"hello": "world"}),
            },
            uuid::Uuid::new_v4(),
        );

        let result = DrainResult {
            messages: vec![msg],
            dropped: 2,
            lagged: 5,
        };

        let json = serialize_envelope(&result).expect("serialization should succeed");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");

        assert_eq!(
            parsed["messages"]
                .as_array()
                .expect("messages is array")
                .len(),
            1
        );
        assert_eq!(parsed["dropped"], 2);
        assert_eq!(parsed["lagged"], 5);
    }

    #[test]
    fn serialize_envelope_empty_messages() {
        let result = DrainResult {
            messages: vec![],
            dropped: 0,
            lagged: 0,
        };

        let json = serialize_envelope(&result).expect("serialization should succeed");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed["messages"].as_array().unwrap().is_empty());
        assert_eq!(parsed["dropped"], 0);
        assert_eq!(parsed["lagged"], 0);
    }

    #[tokio::test]
    async fn subscription_lifecycle_remove_and_reinsert() {
        // Tests the Mutex-drop-before-blocking pattern used by recv_impl:
        // remove receiver from subscriptions, use it, re-insert it.
        let bus = EventBus::new();
        let receiver = bus.subscribe_topic("test.*");

        let mut subscriptions = std::collections::HashMap::new();
        let handle_id: u64 = 42;
        subscriptions.insert(handle_id, receiver);

        // Remove (simulates the lock-drop pattern in recv_impl).
        let mut removed = subscriptions
            .remove(&handle_id)
            .expect("should find handle");
        assert!(
            !subscriptions.contains_key(&handle_id),
            "handle should be gone after remove"
        );

        // Publish while receiver is outside the map.
        publish_ipc_messages(&bus, "test.foo", 3);

        // Drain from the removed receiver — should still work.
        let result = drain_receiver(&mut removed, 10 * 1024 * 1024);
        assert_eq!(result.messages.len(), 3, "receiver should work outside map");

        // Re-insert.
        subscriptions.insert(handle_id, removed);
        assert!(
            subscriptions.contains_key(&handle_id),
            "handle should be back after re-insert"
        );

        // Publish more and verify the re-inserted receiver still works.
        publish_ipc_messages(&bus, "test.bar", 2);
        let receiver = subscriptions.get_mut(&handle_id).unwrap();
        let result = drain_receiver(receiver, 10 * 1024 * 1024);
        assert_eq!(
            result.messages.len(),
            2,
            "receiver should work after re-insert"
        );
    }

    #[tokio::test]
    async fn recv_blocking_wake_plus_drain_burst() {
        // Simulates what recv_impl does: block on recv(), then drain remaining.
        let bus = EventBus::new();
        let mut receiver = bus.subscribe_topic("burst.*");

        let bus_clone = bus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            // Publish 5 messages in a burst.
            publish_ipc_messages(&bus_clone, "burst.test", 5);
        });

        // Block until first message arrives.
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
            .await
            .expect("should not timeout")
            .expect("should get a message");

        // Small delay to let remaining messages arrive in the channel.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Drain remaining (simulates recv_impl's post-wake drain).
        let drain = drain_receiver(&mut receiver, 10 * 1024 * 1024);

        // The first message was consumed by recv(), drain gets the rest.
        let mut total = drain.messages.len() + 1; // +1 for the recv() message
        if let AstridEvent::Ipc { .. } = &*first {
            // first was a matching IPC message, count is correct.
        } else {
            total -= 1;
        }

        assert_eq!(
            total, 5,
            "should get all 5 messages (1 from recv + rest from drain)"
        );
    }
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
