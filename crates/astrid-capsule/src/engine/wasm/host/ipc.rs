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
        .any(|acl| crate::topic::topic_matches(topic_pattern, acl))
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
    if !crate::topic::has_valid_segments(&topic) {
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
        .any(|pattern| crate::topic::topic_matches(&topic, pattern))
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

    // Propagate the principal to the outgoing message. Capsules never
    // touch the principal — it's invisible. Two cases:
    // 1. Invocation context exists (triggered by IPC) → copy from caller
    // 2. No context (uplink publishing from socket) → use capsule's own principal
    // This ensures the principal is ALWAYS set on published messages.
    let principal_str = state
        .caller_context
        .as_ref()
        .and_then(|c| c.principal.clone())
        .unwrap_or_else(|| state.principal.to_string());
    let message = IpcMessage::new(topic, payload, state.capsule_uuid).with_principal(principal_str);

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
    if !crate::topic::has_valid_segments(&topic_pattern) {
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
#[path = "ipc_tests.rs"]
mod tests;

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
