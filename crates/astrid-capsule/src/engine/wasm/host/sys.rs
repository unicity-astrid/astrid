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

    // Write to per-capsule log file. If the invocation principal differs
    // from the capsule's default, open the target principal's log file
    // (open/write/close — no FD caching, avoids leaks in 1000-user deployments).
    let ud2 = user_data.get()?;
    let state2 = ud2
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let level_str = match parsed_level {
        LogLevel::Trace => "TRACE",
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or_else(|_| "0".to_string(), |d| format!("{:.3}", d.as_secs_f64()));

    // Check if invocation principal differs from capsule's default.
    let invocation_principal = state2
        .caller_context
        .as_ref()
        .and_then(|msg| msg.principal.as_deref())
        .and_then(|p| astrid_core::PrincipalId::new(p).ok())
        .filter(|p| *p != state2.principal);

    if let Some(ref inv_principal) = invocation_principal {
        // Cross-principal: open target log file, write, close.
        if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            let ph = home.principal_home(inv_principal);
            let log_dir = ph.log_dir().join(&capsule_id);
            let _ = std::fs::create_dir_all(&log_dir);
            let today = crate::engine::wasm::today_date_string();
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join(format!("{today}.log")))
            {
                use std::io::Write;
                let _ = writeln!(f, "{timestamp} {level_str} [{capsule_id}] {message}");
            }
        }
    } else if let Some(ref log_file) = state2.capsule_log {
        // Default principal: use pre-opened log file (fast path).
        use std::io::Write;
        if let Ok(mut f) = log_file.lock() {
            let _ = writeln!(f, "{timestamp} {level_str} [{capsule_id}] {message}");
        }
    } else {
        drop(state2);
        match parsed_level {
            LogLevel::Trace => tracing::trace!(plugin = %capsule_id, "{message}"),
            LogLevel::Debug => tracing::debug!(plugin = %capsule_id, "{message}"),
            LogLevel::Info => tracing::info!(plugin = %capsule_id, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin = %capsule_id, "{message}"),
            LogLevel::Error => tracing::error!(plugin = %capsule_id, "{message}"),
        }
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

    // Return the raw string value, not JSON-encoded.
    // serde_json::to_string wraps strings in quotes ("\"value\""),
    // causing double-encoding when the SDK's env::var reads it.
    let result = match value {
        Some(serde_json::Value::String(s)) => s,
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

    let result = if let Some(ref msg) = state.caller_context {
        serde_json::json!({
            "principal": msg.principal,
            "source_id": msg.source_id.to_string(),
            "timestamp": msg.timestamp.to_rfc3339(),
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

/// Signal that the capsule's run loop is ready (subscriptions are active).
///
/// Called by the WASM guest after setting up IPC subscriptions. Sends `true`
/// on the readiness watch channel so the kernel can proceed with loading
/// dependent capsules.
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_signal_ready_impl(
    _plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    if let Some(tx) = &state.ready_tx {
        let _ = tx.send(true);
        tracing::debug!(
            capsule = %state.capsule_id,
            "Capsule signaled ready"
        );
    }

    Ok(())
}

/// Returns the current wall-clock time as milliseconds since the UNIX epoch.
///
/// No inputs required. Returns the timestamp as a UTF-8 decimal string.
pub(crate) fn astrid_clock_ms_impl(
    plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u64, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let s = ms.to_string();
    let mem = plugin.memory_new(&s)?;
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
    let host_semaphore = state.host_semaphore.clone();
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
            util::bounded_block_on(&rt_handle, &host_semaphore, async {
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
                            if crate::dispatcher::topic_matches(&request.hook, &interceptor.event) {
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
            });

        // Step 2: Dispatch each interceptor via spawned tasks and collect
        // results. Each invoke_interceptor call may use block_in_place
        // internally, which is safe because it runs in its own spawned task
        // (not nested inside our block_on).
        let responses: Vec<serde_json::Value> =
            util::bounded_block_on(&rt_handle, &host_semaphore, async {
                let mut join_set = tokio::task::JoinSet::new();

                for (capsule, action) in matches {
                    let payload = payload_bytes.clone();
                    let hook = request.hook.clone();
                    join_set.spawn(async move {
                        match capsule.invoke_interceptor(&action, &payload, None) {
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

/// Request payload for cross-capsule capability checks.
#[derive(serde::Deserialize)]
struct CapabilityCheckRequest {
    /// The UUID of the capsule whose capability is being queried.
    source_uuid: String,
    /// The capability to check (e.g. `"allow_prompt_injection"`).
    capability: String,
}

/// Check whether a capsule (identified by its session UUID) has a specific
/// manifest capability.
///
/// Input: JSON `{"source_uuid": "...", "capability": "allow_prompt_injection"}`
/// Output: JSON `{"allowed": true/false}`
///
/// Returns `{"allowed": false}` for unknown UUIDs, unknown capabilities, or
/// if the registry is unavailable (fail-closed).
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_check_capsule_capability_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let request_bytes = util::get_safe_bytes(plugin, &inputs[0], 1024)?;
    let request: CapabilityCheckRequest = serde_json::from_slice(&request_bytes)
        .map_err(|e| Error::msg(format!("invalid capability check request: {e}")))?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let registry = state.capsule_registry.clone();
    let rt_handle = state.runtime_handle.clone();
    let host_semaphore = state.host_semaphore.clone();
    drop(state);

    let allowed = if let Some(registry) = registry {
        if let Ok(source_uuid) = uuid::Uuid::parse_str(&request.source_uuid) {
            util::bounded_block_on(&rt_handle, &host_semaphore, async {
                let reg = registry.read().await;
                let Some(capsule_id) = reg.find_by_uuid(&source_uuid) else {
                    tracing::debug!(
                        uuid = %source_uuid,
                        capability = %request.capability,
                        "UUID not found in registry, denying capability"
                    );
                    return false;
                };
                let Some(capsule) = reg.get(capsule_id) else {
                    return false;
                };
                match request.capability.as_str() {
                    "allow_prompt_injection" => {
                        capsule.manifest().capabilities.allow_prompt_injection
                    },
                    other => {
                        tracing::warn!(
                            capability = %other,
                            "Unknown capability requested, denying"
                        );
                        false
                    },
                }
            })
        } else {
            tracing::debug!(
                uuid = %request.source_uuid,
                "Malformed UUID in capability check, denying"
            );
            false
        }
    } else {
        false
    };

    let result = serde_json::json!({"allowed": allowed}).to_string();
    let mem = plugin.memory_new(&result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}
