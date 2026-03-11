use astrid_core::capsule_abi::LogLevel;
use extism::{CurrentPlugin, Error, UserData, Val};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_log_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let level_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], 64)?;
    let message_bytes: Vec<u8> =
        util::get_safe_bytes(plugin, &inputs[1], util::MAX_LOG_MESSAGE_LEN)?;

    let level = String::from_utf8_lossy(&level_bytes).to_string();
    let message = String::from_utf8_lossy(&message_bytes).to_string();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let capsule_id = state.capsule_id.as_str().to_owned();
    drop(state);

    let parsed_level: LogLevel = match level.to_lowercase().as_str() {
        "trace" => LogLevel::Trace,
        "debug" => LogLevel::Debug,
        "warn" | "warning" => LogLevel::Warn,
        "error" | "err" => LogLevel::Error,
        _ => LogLevel::Info,
    };

    match parsed_level {
        LogLevel::Trace => tracing::trace!(plugin = %capsule_id, "{message}"),
        LogLevel::Debug => tracing::debug!(plugin = %capsule_id, "{message}"),
        LogLevel::Info => tracing::info!(plugin = %capsule_id, "{message}"),
        LogLevel::Warn => tracing::warn!(plugin = %capsule_id, "{message}"),
        LogLevel::Error => tracing::error!(plugin = %capsule_id, "{message}"),
    }

    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_get_config_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let key = String::from_utf8_lossy(&key_bytes).to_string();

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let value = state.config.get(&key).cloned();
    drop(state);

    let result = match value {
        Some(v) => serde_json::to_string(&v).unwrap_or_default(),
        None => String::new(),
    };

    let mem = plugin.memory_new(&result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_get_caller_impl(
    plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let result = if let Some(_msg) = &state.caller_context {
        let session_id = None::<String>; // TODO: extract from AstridEvent
        let user_id = None::<String>; // TODO: extract from AstridEvent
        serde_json::json!({
            "session_id": session_id,
            "user_id": user_id
        })
        .to_string()
    } else {
        String::from("{}")
    };
    drop(state);

    let mem = plugin.memory_new(&result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

/// Trigger request sent by WASM capsules via `hooks::trigger`.
#[derive(serde::Deserialize)]
struct TriggerRequest {
    /// The hook/interceptor topic to fan out (e.g. `before_tool_call`).
    hook: String,
    /// Opaque JSON payload forwarded to each matching interceptor.
    payload: serde_json::Value,
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_trigger_hook_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let event_bytes = util::get_safe_bytes(plugin, &inputs[0], 1024 * 1024)?; // 1MB max payload

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let caller_id = state.capsule_id.clone();
    let registry = state.capsule_registry.clone();
    let rt_handle = state.runtime_handle.clone();
    drop(state);

    let result_bytes = if let Some(registry) = registry {
        // Deserialize the trigger request from the WASM guest.
        let request: TriggerRequest = serde_json::from_slice(&event_bytes)
            .map_err(|e| Error::msg(format!("invalid trigger request: {e}")))?;

        let payload_bytes = serde_json::to_vec(&request.payload).unwrap_or_default();

        // Fan out: find all capsules with interceptors matching the hook topic,
        // invoke each (skipping the caller to prevent infinite recursion),
        // and collect their responses.
        //
        // Step 1: Collect matching capsules under the registry read lock.
        // This happens inside block_in_place → block_on so we can acquire
        // the async RwLock, but we do NOT call invoke_interceptor here
        // (which itself does block_in_place and would panic if nested).
        let matches: Vec<(std::sync::Arc<dyn crate::capsule::Capsule>, String)> =
            tokio::task::block_in_place(|| {
                rt_handle.block_on(async {
                    let registry = registry.read().await;
                    let mut matches = Vec::new();

                    for capsule_id in registry.list() {
                        // Skip the calling capsule to prevent recursion.
                        if *capsule_id == caller_id {
                            continue;
                        }
                        if let Some(capsule) = registry.get(capsule_id) {
                            if !matches!(capsule.state(), crate::capsule::CapsuleState::Ready) {
                                continue;
                            }
                            for interceptor in &capsule.manifest().interceptors {
                                if crate::dispatcher::topic_matches(
                                    &request.hook,
                                    &interceptor.event,
                                ) {
                                    matches.push((
                                        std::sync::Arc::clone(&capsule),
                                        interceptor.action.clone(),
                                    ));
                                }
                            }
                        }
                    }
                    matches
                    // Read lock dropped here.
                })
            });

        // Step 2: Dispatch each interceptor via spawned tasks and collect
        // results. Each invoke_interceptor call may use block_in_place
        // internally, which is safe because it runs in its own spawned task
        // (not nested inside our block_on).
        let responses: Vec<serde_json::Value> = tokio::task::block_in_place(|| {
            rt_handle.block_on(async {
                let mut join_set = tokio::task::JoinSet::new();

                for (capsule, action) in matches {
                    let payload = payload_bytes.clone();
                    let hook = request.hook.clone();
                    join_set.spawn(async move {
                        match capsule.invoke_interceptor(&action, &payload) {
                            Ok(bytes) if bytes.is_empty() => None,
                            Ok(bytes) => {
                                match serde_json::from_slice::<serde_json::Value>(&bytes) {
                                    Ok(val) => Some(val),
                                    Err(_) => {
                                        tracing::warn!(
                                            capsule_id = %capsule.id(),
                                            action = %action,
                                            "interceptor returned non-JSON response, skipping"
                                        );
                                        None
                                    },
                                }
                            },
                            Err(e) => {
                                tracing::warn!(
                                    capsule_id = %capsule.id(),
                                    action = %action,
                                    hook = %hook,
                                    error = %e,
                                    "interceptor invocation failed during hook trigger"
                                );
                                None
                            },
                        }
                    });
                }

                let mut responses = Vec::new();
                while let Some(result) = join_set.join_next().await {
                    if let Ok(Some(val)) = result {
                        responses.push(val);
                    }
                }
                responses
            })
        });

        match serde_json::to_vec(&responses) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize hook responses");
                b"[]".to_vec()
            },
        }
    } else {
        // No registry available — return empty array (no subscribers).
        b"[]".to_vec()
    };

    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}
