use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_events::AstridEvent;
use astrid_events::EventMetadata;
use astrid_events::EventReceiver;
use astrid_events::ipc::{IpcMessage, IpcPayload};
use extism::{CurrentPlugin, Error, UserData, Val};

// ── Extracted testable core ─────────────────────────────────────────

/// Check whether a subscription topic pattern is allowed by the capsule's
/// declared `ipc_subscribe` ACL patterns. Returns `Ok(())` if allowed,
/// or `Err(reason)` if denied.
pub(crate) fn check_subscribe_acl(
    capsule_id: &str,
    topic_pattern: &str,
    acl_patterns: &[String],
) -> Result<(), String> {
    if acl_patterns.is_empty() {
        return Err(format!(
            "Capsule '{capsule_id}' has no ipc_subscribe declarations - \
             subscribing is denied. Add ipc_subscribe patterns to Capsule.toml [capabilities]"
        ));
    }

    // NOTE: argument order is intentional. topic_matches(topic, pattern) checks
    // whether `topic` (here: the subscription request) falls within `pattern`
    // (here: the ACL entry). This means:
    //   subscribe("foo.bar") vs ACL "foo.*" -> topic_matches("foo.bar", "foo.*") = true
    //   subscribe("foo.*")   vs ACL "foo.bar" -> topic_matches("foo.*", "foo.bar") = false
    // The second case correctly prevents scope escalation via wildcard subscriptions.
    if !acl_patterns
        .iter()
        .any(|acl| crate::dispatcher::topic_matches(topic_pattern, acl))
    {
        return Err(format!(
            "Capsule '{capsule_id}' is not allowed to subscribe to topic \
             '{topic_pattern}' - declared ipc_subscribe patterns: {acl_patterns:?}"
        ));
    }

    Ok(())
}

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
            let msg_len = serde_json::to_vec(&message.payload)
                .map(|v| v.len())
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

/// Remove a subscription by handle ID, rejecting runtime-owned interceptor handles.
///
/// Returns `Err` if the handle is protected (auto-subscribed interceptor) or
/// if the handle ID is not found in `subscriptions`.
pub(crate) fn remove_subscription(
    subscriptions: &mut std::collections::HashMap<u64, EventReceiver>,
    is_protected: bool,
    handle_id: u64,
) -> Result<(), Error> {
    if is_protected {
        tracing::warn!(
            handle_id,
            "Guest attempted to unsubscribe a runtime-owned interceptor handle",
        );
        return Err(Error::msg(
            "Cannot unsubscribe a runtime-owned interceptor handle",
        ));
    }

    if subscriptions.remove(&handle_id).is_none() {
        return Err(Error::msg("Subscription handle not found"));
    }

    Ok(())
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

    // Deserialize the guest payload into an IpcPayload, falling back to
    // Custom for unrecognised or missing type tags.  See IpcPayload::from_json_value
    // for the rationale behind the pre-check.
    let payload = match serde_json::from_slice::<serde_json::Value>(&payload_bytes) {
        Ok(data) => IpcPayload::from_json_value(data),
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

    // Enforce IPC topic subscription restrictions from Capsule.toml.
    // Fail-closed: capsules without ipc_subscribe declarations cannot subscribe.
    check_subscribe_acl(
        state.capsule_id.as_ref(),
        &topic_pattern,
        &state.ipc_subscribe_patterns,
    )
    .map_err(Error::msg)?;

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
    let (mut receiver, runtime_handle, cancel_token, host_semaphore) = {
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        let receiver = state
            .subscriptions
            .remove(&handle_id)
            .ok_or_else(|| Error::msg("Subscription handle not found"))?;
        let runtime_handle = state.runtime_handle.clone();
        let cancel_token = state.cancel_token.clone();
        let host_semaphore = state.host_semaphore.clone();
        (receiver, runtime_handle, cancel_token, host_semaphore)
    };

    // Block the WASM thread until a message arrives, timeout expires, or the
    // capsule is unloaded (cancellation). Routed through the host semaphore to
    // bound concurrent blocking operations across all capsules.
    //
    // Note: the helper uses a biased select that strictly prioritises
    // cancellation over completion. If a message arrives in the same poll
    // tick as cancellation, the message is discarded. This is acceptable
    // during teardown and prevents delayed shutdown under high throughput.
    let event = util::bounded_block_on_cancellable(
        &runtime_handle,
        &host_semaphore,
        &cancel_token,
        async {
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                receiver.recv(),
            )
            .await
            .ok()
            .flatten()
        },
    )
    .flatten();

    // Collect the blocking-wake message (if any) plus drain remaining.
    let mut drain = drain_receiver(&mut receiver, util::MAX_GUEST_PAYLOAD_LEN as usize);

    // Prepend the message that woke us (it was consumed by recv, not try_recv).
    if let Some(event) = event
        && let AstridEvent::Ipc { message, .. } = &*event
    {
        drain.messages.insert(0, message.clone());
    }

    // Re-insert the receiver after draining. During teardown (cancel token
    // fired), skip re-insertion: the capsule is dying and the lock may be
    // poisoned from concurrent cleanup, which would surface a misleading error.
    if !cancel_token.is_cancelled() {
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

    /// Thin wrapper around the production deserialization path so we can
    /// test it without a full WASM plugin context.
    fn deserialize_publish_payload(payload_bytes: &[u8]) -> Result<IpcPayload, String> {
        serde_json::from_slice::<serde_json::Value>(payload_bytes)
            .map(IpcPayload::from_json_value)
            .map_err(|_| "IPC payload is not valid JSON".into())
    }

    #[test]
    fn publish_unknown_type_tag_produces_custom() {
        let input = serde_json::json!({"type": "my_plugin_msg", "foo": 1});
        let bytes = serde_json::to_vec(&input).unwrap();
        let payload = deserialize_publish_payload(&bytes).unwrap();

        match payload {
            IpcPayload::Custom { data } => {
                assert_eq!(data["type"], "my_plugin_msg");
                assert_eq!(data["foo"], 1);
            },
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn publish_missing_type_tag_produces_custom() {
        let input = serde_json::json!({"foo": 1, "bar": "baz"});
        let bytes = serde_json::to_vec(&input).unwrap();
        let payload = deserialize_publish_payload(&bytes).unwrap();

        match payload {
            IpcPayload::Custom { data } => {
                assert_eq!(data["foo"], 1);
            },
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn publish_non_object_payload_produces_custom() {
        // A bare JSON string has no `type` field.
        let bytes = br#""hello""#;
        let payload = deserialize_publish_payload(bytes).unwrap();

        match payload {
            IpcPayload::Custom { data } => {
                assert_eq!(data, "hello");
            },
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn publish_known_type_tag_deserializes_correctly() {
        let input = serde_json::json!({"type": "connect"});
        let bytes = serde_json::to_vec(&input).unwrap();
        let payload = deserialize_publish_payload(&bytes).unwrap();

        assert_eq!(payload, IpcPayload::Connect);
    }

    #[test]
    fn publish_known_tag_with_malformed_fields_falls_to_custom() {
        // `user_input` requires a `text` field. Without it, deserialization
        // fails and the .unwrap_or path produces Custom.
        let input = serde_json::json!({"type": "user_input"});
        let bytes = serde_json::to_vec(&input).unwrap();
        let payload = deserialize_publish_payload(&bytes).unwrap();

        match payload {
            IpcPayload::Custom { data } => {
                assert_eq!(data["type"], "user_input");
            },
            other => panic!("expected Custom (malformed known tag), got {other:?}"),
        }
    }

    #[test]
    fn publish_custom_tag_with_data_unwraps_inner_value() {
        // A well-formed {"type": "custom", "data": {...}} should deserialize
        // via serde so that `data` is the inner value, not the outer wrapper.
        let input = serde_json::json!({"type": "custom", "data": {"foo": 1}});
        let bytes = serde_json::to_vec(&input).unwrap();
        let payload = deserialize_publish_payload(&bytes).unwrap();

        match payload {
            IpcPayload::Custom { data } => {
                assert_eq!(data, serde_json::json!({"foo": 1}));
            },
            other => panic!("expected Custom with inner data, got {other:?}"),
        }
    }

    #[test]
    fn publish_invalid_json_is_error() {
        let result = deserialize_publish_payload(b"not json at all");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cancellation_token_unblocks_recv() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe_topic("cancel.test");
        let cancel_token = tokio_util::sync::CancellationToken::new();

        let token = cancel_token.clone();

        // Cancel after 50ms.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            token.cancel();
        });

        // Block on recv with a long timeout. Cancellation should unblock it
        // well before the 60-second timeout.
        let start = std::time::Instant::now();
        let event = tokio::select! {
            result = tokio::time::timeout(
                std::time::Duration::from_millis(60_000),
                receiver.recv(),
            ) => result.ok().flatten(),
            () = cancel_token.cancelled() => None,
        };

        let elapsed = start.elapsed();
        assert!(event.is_none(), "should return None on cancellation");
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "should unblock promptly, took {elapsed:?}"
        );
    }

    #[test]
    fn protected_interceptor_handle_rejects_unsubscribe() {
        // Simulate post-auto-subscribe state: handle 1 is in subscriptions
        // and flagged as protected (runtime-owned interceptor).
        let bus = EventBus::new();
        let receiver = bus.subscribe_topic("interceptor.topic");

        let mut subscriptions = std::collections::HashMap::new();
        let handle_id: u64 = 1;
        subscriptions.insert(handle_id, receiver);

        // The production function must reject protected handles.
        let result = remove_subscription(&mut subscriptions, true, handle_id);
        assert!(
            result.is_err(),
            "should reject unsubscribe on protected handle",
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("runtime-owned interceptor handle"),
            "error should mention interceptor handle",
        );

        // Subscription must still exist.
        assert!(
            subscriptions.contains_key(&handle_id),
            "protected subscription must survive unsubscribe attempt",
        );
    }

    #[test]
    fn guest_handle_allows_unsubscribe() {
        // Guest-created handle (not protected) can be removed.
        let bus = EventBus::new();
        let receiver = bus.subscribe_topic("guest.topic");

        let mut subscriptions = std::collections::HashMap::new();
        let handle_id: u64 = 99;
        subscriptions.insert(handle_id, receiver);

        // The production function must allow unprotected handles.
        let result = remove_subscription(&mut subscriptions, false, handle_id);
        assert!(result.is_ok(), "should allow unsubscribe on guest handle");
        assert!(
            !subscriptions.contains_key(&handle_id),
            "subscription should be gone after removal",
        );
    }

    #[test]
    fn unsubscribe_nonexistent_handle_returns_not_found() {
        let mut subscriptions = std::collections::HashMap::new();

        let result = remove_subscription(&mut subscriptions, false, 42);
        assert!(result.is_err(), "should reject nonexistent handle");
        assert!(
            result.unwrap_err().to_string().contains("not found"),
            "error should mention not found",
        );
    }

    // ── subscribe ACL tests ────────────────────────────────────────

    #[test]
    fn subscribe_acl_empty_patterns_denies() {
        let err = check_subscribe_acl("test-capsule", "any.topic", &[]).unwrap_err();
        assert!(err.contains("no ipc_subscribe declarations"));
    }

    #[test]
    fn subscribe_acl_exact_match_allows() {
        let patterns = vec!["agent.v1.response".into()];
        assert!(check_subscribe_acl("test", "agent.v1.response", &patterns).is_ok());
    }

    #[test]
    fn subscribe_acl_wildcard_matches_concrete() {
        let patterns = vec!["registry.v1.*".into()];
        assert!(check_subscribe_acl("test", "registry.v1.providers", &patterns).is_ok());
    }

    #[test]
    fn subscribe_acl_segment_count_mismatch_denies() {
        // ACL has 3 segments, subscription has 4 - topic_matches requires equal count.
        let patterns = vec!["registry.v1.*".into()];
        let err =
            check_subscribe_acl("test", "registry.v1.selection.callback", &patterns).unwrap_err();
        assert!(err.contains("not allowed to subscribe"));
    }

    #[test]
    fn subscribe_acl_wildcard_subscription_matches_wildcard_acl() {
        // A capsule with ACL "foo.*" is explicitly authorizing a "foo.*" subscription
        // (same pattern). The ACL check gates the subscription string, not individual
        // messages.
        let patterns = vec!["foo.*".into()];
        assert!(check_subscribe_acl("test", "foo.*", &patterns).is_ok());
    }

    #[test]
    fn subscribe_acl_wildcard_subscription_denied_by_exact_acl() {
        // A capsule with ACL ["foo.bar"] must NOT be able to subscribe to "foo.*"
        // which would receive all foo.* events, not just foo.bar. This is the
        // primary scope-escalation prevention invariant.
        let patterns = vec!["foo.bar".into()];
        let result = check_subscribe_acl("test", "foo.*", &patterns);
        assert!(
            result.is_err(),
            "wildcard subscription must be denied by exact ACL"
        );
    }

    #[test]
    fn subscribe_acl_unrelated_topic_denies() {
        let patterns = vec!["agent.v1.*".into()];
        let err = check_subscribe_acl("test", "session.v1.clear", &patterns).unwrap_err();
        assert!(err.contains("not allowed to subscribe"));
    }

    #[test]
    fn subscribe_acl_multiple_patterns_second_matches() {
        let patterns = vec!["agent.v1.*".into(), "session.v1.response.*".into()];
        assert!(check_subscribe_acl("test", "session.v1.response.abc", &patterns).is_ok());
    }

    #[test]
    fn subscribe_acl_malformed_pattern_silently_denies() {
        // A malformed ACL pattern (empty segments) causes topic_matches to
        // return false via has_valid_segments, resulting in a silent deny.
        // This is safe (fail-closed) but the error message is misleading.
        // See #374 for load-time validation of ipc_subscribe patterns.
        let patterns = vec!["foo..bar".into()];
        let result = check_subscribe_acl("test", "foo.x.bar", &patterns);
        assert!(
            result.is_err(),
            "malformed ACL pattern must not allow subscriptions"
        );
    }
}

/// Return the pre-registered interceptor handle mappings for run-loop capsules.
///
/// Called by the WASM guest at startup to discover which IPC subscription
/// handles correspond to interceptor actions. Returns a JSON array of
/// `InterceptorHandle` objects, or an empty array if no interceptors are
/// auto-subscribed.
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_get_interceptor_handles_impl(
    plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let json = serde_json::to_string(&state.interceptor_handles)
        .map_err(|e| Error::msg(format!("failed to serialize interceptor handles: {e}")))?;

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

    let is_protected = state
        .interceptor_handles
        .iter()
        .any(|h| h.handle_id == handle_id);
    remove_subscription(&mut state.subscriptions, is_protected, handle_id)
}
