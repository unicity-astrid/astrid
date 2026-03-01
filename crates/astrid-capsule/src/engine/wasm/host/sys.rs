use astrid_core::plugin_abi::LogLevel;
use extism::{CurrentPlugin, Error, UserData, Val};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

#[allow(clippy::needless_pass_by_value)]
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

#[allow(clippy::needless_pass_by_value)]
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

#[allow(clippy::needless_pass_by_value)]
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

#[allow(clippy::needless_pass_by_value)]
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

    // In Phase 7, the HookManager is passed into HostState so we can execute hooks synchronously.
    let result_bytes = if let Some(_hook_manager) = &state.hook_manager {
        // We use block_in_place because Extism host functions are synchronous,
        // but our HookManager executes shell scripts/HTTP asynchronously.
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Here we would actually deserialize the event_bytes and trigger the hook.
                // For now, we return a pass-through success result.
                Ok::<Vec<u8>, String>(event_bytes.to_vec())
            })
        });
        result.unwrap_or_else(|e| format!(r#"{{"error": "{}"}}"#, e).into_bytes())
    } else {
        // No hook manager configured, just pass through
        event_bytes.to_vec()
    };

    drop(state);

    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}
