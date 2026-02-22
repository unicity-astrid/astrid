use astrid_core::plugin_abi::LogLevel;
use extism::{CurrentPlugin, Error, UserData, Val};

use crate::wasm::host::util;
use crate::wasm::host_state::HostState;

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_log_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let level: String = util::get_safe_string(plugin, &inputs[0], 64)?;
    let message: String = util::get_safe_string(plugin, &inputs[1], util::MAX_LOG_MESSAGE_LEN)?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    drop(state);

    let parsed_level: LogLevel = match level.to_lowercase().as_str() {
        "trace" => LogLevel::Trace,
        "debug" => LogLevel::Debug,
        "warn" | "warning" => LogLevel::Warn,
        "error" | "err" => LogLevel::Error,
        _ => LogLevel::Info,
    };

    match parsed_level {
        LogLevel::Trace => tracing::trace!(plugin = %plugin_id, "{message}"),
        LogLevel::Debug => tracing::debug!(plugin = %plugin_id, "{message}"),
        LogLevel::Info => tracing::info!(plugin = %plugin_id, "{message}"),
        LogLevel::Warn => tracing::warn!(plugin = %plugin_id, "{message}"),
        LogLevel::Error => tracing::error!(plugin = %plugin_id, "{message}"),
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
    let key: String = util::get_safe_string(plugin, &inputs[0], util::MAX_KEY_LEN)?;

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
