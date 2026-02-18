//! Extism host function implementations matching the WIT `host` interface.
//!
//! Fourteen host functions are registered with every Extism plugin instance:
//!
//! | Function | Security Gate | Async Bridge |
//! |----------|--------------|--------------|
//! | `astrid_channel_send` | No | No |
//! | `astrid_fs_exists` | Yes | No |
//! | `astrid_fs_mkdir` | Yes | No |
//! | `astrid_fs_readdir` | Yes | No |
//! | `astrid_fs_stat` | Yes | No |
//! | `astrid_fs_unlink` | Yes | No |
//! | `astrid_get_config` | No | No |
//! | `astrid_http_request` | Yes | Yes |
//! | `astrid_kv_get` | No | Yes |
//! | `astrid_kv_set` | No | Yes |
//! | `astrid_log` | No | No |
//! | `astrid_read_file` | Yes | Yes |
//! | `astrid_register_connector` | Yes | Yes |
//! | `astrid_write_file` | Yes | Yes |
//!
//! All host functions use `UserData<HostState>` for shared state access.
//! Async operations are bridged via `Handle::block_on()` — this requires
//! the **multi-threaded** tokio runtime.

use std::path::Path;

use extism::{CurrentPlugin, Error, PTR, UserData, Val};

#[cfg(feature = "http")]
use astrid_core::plugin_abi::HttpResponse;
use astrid_core::plugin_abi::{KeyValuePair, LogLevel};

use super::host_state::HostState;

/// Maximum inbound message content size (1 MB).
const MAX_INBOUND_MESSAGE_BYTES: usize = 1_048_576;

/// Maximum length for connector name and `platform_user_id` strings (256 chars).
const MAX_STRING_LENGTH: usize = 256;

// ---------------------------------------------------------------------------
// astrid_channel_send(connector_id, platform_user_id, content) -> result_json
// ---------------------------------------------------------------------------

/// Parse a platform string into a [`FrontendType`](astrid_core::identity::FrontendType).
///
/// Accepts lowercase platform names (e.g. `"discord"`, `"telegram"`, `"cli"`)
/// and maps unknown strings to `FrontendType::Custom(...)`.
///
/// Note: unlike [`parse_connector_profile`], this intentionally never errors.
/// `FrontendType` has a `Custom` variant for extensibility, so unknown
/// platforms are valid rather than rejected.
fn parse_frontend_type(platform: &str) -> astrid_core::identity::FrontendType {
    use astrid_core::identity::FrontendType;
    match platform.to_lowercase().as_str() {
        "discord" => FrontendType::Discord,
        "whatsapp" | "whats_app" => FrontendType::WhatsApp,
        "telegram" => FrontendType::Telegram,
        "slack" => FrontendType::Slack,
        "web" => FrontendType::Web,
        "cli" => FrontendType::Cli,
        other => FrontendType::Custom(other.to_string()),
    }
}

/// Parse a profile string into a [`ConnectorProfile`](astrid_core::ConnectorProfile).
///
/// Returns `Err` for unknown profile strings.
fn parse_connector_profile(profile: &str) -> Result<astrid_core::ConnectorProfile, Error> {
    use astrid_core::ConnectorProfile;
    match profile.to_lowercase().as_str() {
        "chat" => Ok(ConnectorProfile::Chat),
        "interactive" => Ok(ConnectorProfile::Interactive),
        "notify" => Ok(ConnectorProfile::Notify),
        "bridge" => Ok(ConnectorProfile::Bridge),
        other => Err(Error::msg(format!(
            "invalid connector profile: {other:?} (expected: chat, interactive, notify, bridge)"
        ))),
    }
}

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_channel_send_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let connector_id_str: String = plugin.memory_get_val(&inputs[0])?;
    let platform_user_id: String = plugin.memory_get_val(&inputs[1])?;
    let content: String = plugin.memory_get_val(&inputs[2])?;

    // Validate connector_id length (UUIDs are 36 chars; reject oversized strings early)
    if connector_id_str.len() > 64 {
        return Err(Error::msg("connector_id too long"));
    }

    // Validate platform_user_id length
    if platform_user_id.len() > MAX_STRING_LENGTH {
        return Err(Error::msg(format!(
            "platform_user_id too long: {} bytes (max {MAX_STRING_LENGTH})",
            platform_user_id.len()
        )));
    }

    // Validate content size (max 1 MB per #36 guidance)
    if content.len() > MAX_INBOUND_MESSAGE_BYTES {
        return Err(Error::msg(format!(
            "inbound message content too large: {} bytes (max {})",
            content.len(),
            MAX_INBOUND_MESSAGE_BYTES
        )));
    }

    // Parse the connector_id UUID
    let connector_uuid: uuid::Uuid = connector_id_str
        .parse()
        .map_err(|e| Error::msg(format!("invalid connector_id: {e}")))?;
    let connector_id = astrid_core::ConnectorId::from_uuid(connector_uuid);

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let inbound_tx = state.inbound_tx.clone();

    // Explicit capability check (defense-in-depth — inbound_tx would be None
    // anyway for non-connector plugins, but an explicit check is clearer)
    if !state.has_connector_capability {
        return Err(Error::msg(format!(
            "plugin {plugin_id} does not declare Connector capability"
        )));
    }

    // Find the platform from the registered connector
    let platform = state
        .registered_connectors
        .iter()
        .find(|c| c.id == connector_id)
        .map(|c| c.frontend_type.clone())
        .ok_or_else(|| {
            Error::msg(format!(
                "connector {connector_id} not registered by plugin {plugin_id}"
            ))
        })?;
    drop(state);

    // Verify we have an inbound channel
    let tx = inbound_tx.ok_or_else(|| {
        Error::msg(format!(
            "plugin {plugin_id} has no inbound channel — is Connector capability declared?"
        ))
    })?;

    // Build the inbound message
    let message =
        astrid_core::InboundMessage::builder(connector_id, platform, platform_user_id, content)
            .build();

    // Send through bounded channel — report drop if full (per #36 guidance)
    let result = match tx.try_send(message) {
        Ok(()) => serde_json::json!({"ok": true}),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            tracing::warn!(
                plugin = %plugin_id,
                connector = %connector_id,
                "inbound channel full — dropping message"
            );
            serde_json::json!({"ok": false, "dropped": true})
        },
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            return Err(Error::msg("inbound channel closed"));
        },
    };

    let result = result.to_string();
    let mem = plugin.memory_new(&result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_log(level, message)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_log_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let level: String = plugin.memory_get_val(&inputs[0])?;
    let message: String = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    drop(state);

    let parsed_level: LogLevel =
        serde_json::from_str(&format!("\"{level}\"")).unwrap_or(LogLevel::Info);

    match parsed_level {
        LogLevel::Trace => tracing::trace!(plugin = %plugin_id, "{message}"),
        LogLevel::Debug => tracing::debug!(plugin = %plugin_id, "{message}"),
        LogLevel::Info => tracing::info!(plugin = %plugin_id, "{message}"),
        LogLevel::Warn => tracing::warn!(plugin = %plugin_id, "{message}"),
        LogLevel::Error => tracing::error!(plugin = %plugin_id, "{message}"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_fs_exists(path) -> "true" | "false"
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_fs_exists_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let workspace_root = state.workspace_root.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let exists = resolved.exists();

    let result = if exists { "true" } else { "false" };
    let mem = plugin.memory_new(result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_fs_mkdir(path)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_fs_mkdir_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied mkdir: {reason}")));
        }
    }

    std::fs::create_dir_all(&resolved)
        .map_err(|e| Error::msg(format!("mkdir failed ({resolved_str}): {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_fs_readdir(path) -> JSON array of filenames
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_fs_readdir_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied readdir: {reason}")));
        }
    }

    let entries: Vec<String> = std::fs::read_dir(&resolved)
        .map_err(|e| Error::msg(format!("readdir failed ({resolved_str}): {e}")))?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();

    let json = serde_json::to_string(&entries)
        .map_err(|e| Error::msg(format!("failed to serialize readdir result: {e}")))?;

    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_fs_stat(path) -> JSON {size, isDir, mtime}
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_fs_stat_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied stat: {reason}")));
        }
    }

    let metadata = std::fs::metadata(&resolved)
        .map_err(|e| Error::msg(format!("stat failed ({resolved_str}): {e}")))?;

    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0u64, |d| d.as_secs());

    let stat = serde_json::json!({
        "size": metadata.len(),
        "isDir": metadata.is_dir(),
        "mtime": mtime
    });

    let json = stat.to_string();
    let mem = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_fs_unlink(path)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_fs_unlink_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied unlink: {reason}")));
        }
    }

    std::fs::remove_file(&resolved)
        .map_err(|e| Error::msg(format!("unlink failed ({resolved_str}): {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_get_config(key) -> value_json
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_get_config_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;

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

// ---------------------------------------------------------------------------
// astrid_kv_get(key) -> value
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_kv_get_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let kv = state.kv.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let result = handle.block_on(async { kv.get(&key).await });

    let value = match result {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(None) => String::new(),
        Err(e) => return Err(Error::msg(format!("kv_get failed: {e}"))),
    };

    let mem = plugin.memory_new(&value)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_kv_set(key, value)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_kv_set_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;
    let value: String = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let kv = state.kv.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let result = handle.block_on(async { kv.set(&key, value.into_bytes()).await });

    match result {
        Ok(()) => Ok(()),
        Err(e) => Err(Error::msg(format!("kv_set failed: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// astrid_read_file(path) -> content
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_read_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    // Resolve and confine path to workspace
    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    // Security check
    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file read: {reason}")));
        }
    }

    // Read file
    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| Error::msg(format!("read_file failed ({resolved_str}): {e}")))?;

    let mem = plugin.memory_new(&content)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_register_connector(name, platform, profile) -> connector_id
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_register_connector_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let name: String = plugin.memory_get_val(&inputs[0])?;
    let platform_str: String = plugin.memory_get_val(&inputs[1])?;
    let profile_str: String = plugin.memory_get_val(&inputs[2])?;

    // Validate string lengths and emptiness
    if name.is_empty() {
        return Err(Error::msg("connector name must not be empty"));
    }
    if name.len() > MAX_STRING_LENGTH {
        return Err(Error::msg(format!(
            "connector name too long: {} bytes (max {MAX_STRING_LENGTH})",
            name.len()
        )));
    }
    if platform_str.trim().is_empty() {
        return Err(Error::msg(
            "platform name must not be empty or whitespace-only",
        ));
    }
    if platform_str.len() > MAX_STRING_LENGTH {
        return Err(Error::msg(format!(
            "platform string too long: {} bytes (max {MAX_STRING_LENGTH})",
            platform_str.len()
        )));
    }
    if profile_str.len() > MAX_STRING_LENGTH {
        return Err(Error::msg(format!(
            "profile string too long: {} bytes (max {MAX_STRING_LENGTH})",
            profile_str.len()
        )));
    }

    // Validate inputs — trim whitespace before parsing so the stored
    // FrontendType never contains leading/trailing spaces.
    let frontend_type = parse_frontend_type(platform_str.trim());
    let profile = parse_connector_profile(&profile_str)?;

    // First lock: read capability flag, security gate, and runtime handle
    let ud = user_data.get()?;
    let (plugin_id, has_capability, security, handle) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        (
            state.plugin_id.as_str().to_owned(),
            state.has_connector_capability,
            state.security.clone(),
            state.runtime_handle.clone(),
        )
    };

    // Gate: only plugins with Connector capability may register connectors
    if !has_capability {
        return Err(Error::msg(format!(
            "plugin {plugin_id} does not declare Connector capability"
        )));
    }

    // Security gate check (may block on async)
    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let cname = name.clone();
        let plat = platform_str.clone();
        let check = handle
            .block_on(async move { gate.check_connector_register(&pid, &cname, &plat).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied connector registration: {reason}"
            )));
        }
    }

    // Build the connector source (validated via ConnectorSource::new_wasm)
    let source = astrid_core::ConnectorSource::new_wasm(&plugin_id).map_err(|e| {
        Error::msg(format!(
            "failed to create connector source for plugin {plugin_id}: {e}"
        ))
    })?;

    // Build the descriptor
    //
    // NOTE: Capabilities are hardcoded to `receive_only()` for Phase 3. WASM
    // connector plugins are inbound-only; send capabilities will be added in
    // a future phase when outbound message routing is implemented.
    let descriptor = astrid_core::ConnectorDescriptor::builder(name, frontend_type)
        .source(source)
        .capabilities(astrid_core::ConnectorCapabilities::receive_only())
        .profile(profile)
        .build();

    let connector_id = descriptor.id.to_string();

    // Second lock: register the descriptor
    {
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        state.register_connector(descriptor).map_err(|e| {
            Error::msg(format!(
                "plugin {plugin_id}: {e} (max {})",
                super::host_state::MAX_CONNECTORS_PER_PLUGIN
            ))
        })?;
    }

    tracing::info!(
        plugin = %plugin_id,
        connector = %connector_id,
        "registered connector"
    );

    let mem = plugin.memory_new(&connector_id)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_write_file(path, content)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_write_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;
    let content: String = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    // Resolve and confine path to workspace
    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    // Security check
    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file write: {reason}")));
        }
    }

    // Write file
    std::fs::write(&resolved, content.as_bytes())
        .map_err(|e| Error::msg(format!("write_file failed ({resolved_str}): {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_http_request(request_json) -> response_json
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_http_request_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct HttpRequest {
        method: String,
        url: String,
        #[serde(default)]
        headers: Vec<KeyValuePair>,
        #[serde(default)]
        body: Option<String>,
    }

    let request_json: String = plugin.memory_get_val(&inputs[0])?;

    let req: HttpRequest = serde_json::from_str(&request_json)
        .map_err(|e| Error::msg(format!("invalid HTTP request JSON: {e}")))?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    // Security check
    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let method = req.method.clone();
        let url = req.url.clone();
        let check =
            handle.block_on(async move { gate.check_http_request(&pid, &method, &url).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied HTTP request: {reason}"
            )));
        }
    }

    // Perform the HTTP request (feature-gated)
    #[cfg(feature = "http")]
    {
        let response = handle.block_on(async {
            perform_http_request(&req.method, &req.url, &req.headers, req.body.as_deref()).await
        })?;
        let response_json = serde_json::to_string(&response)
            .map_err(|e| Error::msg(format!("failed to serialize HTTP response: {e}")))?;
        let mem = plugin.memory_new(&response_json)?;
        outputs[0] = plugin.memory_to_val(mem);
        Ok(())
    }

    #[cfg(not(feature = "http"))]
    {
        let _ = outputs;
        Err(Error::msg(
            "HTTP support not enabled — enable the 'http' feature on astrid-plugins",
        ))
    }
}

// ---------------------------------------------------------------------------
// HTTP implementation (behind feature flag)
// ---------------------------------------------------------------------------

#[cfg(feature = "http")]
async fn perform_http_request(
    method: &str,
    url: &str,
    headers: &[KeyValuePair],
    body: Option<&str>,
) -> Result<HttpResponse, Error> {
    let client = reqwest::Client::new();
    let mut builder = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        "HEAD" => client.head(url),
        other => {
            return Err(Error::msg(format!("unsupported HTTP method: {other}")));
        },
    };

    for kv in headers {
        builder = builder.header(&kv.key, &kv.value);
    }

    if let Some(b) = body {
        builder = builder.body(b.to_string());
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| Error::msg(format!("HTTP request failed: {e}")))?;

    let status = resp.status().as_u16();
    let resp_headers: Vec<KeyValuePair> = resp
        .headers()
        .iter()
        .map(|(k, v)| KeyValuePair {
            key: k.to_string(),
            value: v.to_str().unwrap_or("").to_string(),
        })
        .collect();
    let resp_body = resp
        .text()
        .await
        .map_err(|e| Error::msg(format!("failed to read HTTP response body: {e}")))?;

    Ok(HttpResponse {
        status,
        headers: resp_headers,
        body: resp_body,
    })
}

// ---------------------------------------------------------------------------
// Workspace path resolution
// ---------------------------------------------------------------------------

/// Resolve a plugin-provided path relative to the workspace root and verify
/// it does not escape the workspace boundary.
///
/// - Relative paths are joined onto `workspace_root`
/// - Absolute paths are used as-is
/// - The resulting canonical path must start with the canonical workspace root
fn resolve_within_workspace(
    workspace_root: &Path,
    requested: &str,
) -> Result<std::path::PathBuf, Error> {
    // Canonicalize the root first so all comparisons use the real path.
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let requested_path = Path::new(requested);
    // Always join relative to canonical root for consistent comparison.
    let joined = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        canonical_root.join(requested_path)
    };

    // For existing paths, canonicalize fully (resolves symlinks).
    // For non-existing paths, canonicalize the parent if possible.
    let canonical_path = if joined.exists() {
        joined
            .canonicalize()
            .map_err(|e| Error::msg(format!("failed to resolve path: {e}")))?
    } else {
        // Canonicalize parent directory if it exists, then append the filename.
        let parent = joined.parent().unwrap_or(&joined);
        let filename = joined.file_name();
        if parent.exists() {
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| Error::msg(format!("failed to resolve parent: {e}")))?;
            match filename {
                Some(name) => canonical_parent.join(name),
                None => canonical_parent,
            }
        } else {
            // Neither path nor parent exist — do a lexical check.
            // The write will fail anyway if the directory doesn't exist.
            lexical_normalize(&joined)
        }
    };

    if !canonical_path.starts_with(&canonical_root) {
        return Err(Error::msg(format!(
            "path escapes workspace boundary: {requested} resolves to {}",
            canonical_path.display()
        )));
    }

    Ok(canonical_path)
}

/// Lexically normalize a path (resolve `.` and `..` without filesystem access).
fn lexical_normalize(path: &Path) -> std::path::PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                }
            },
            std::path::Component::CurDir => {},
            other => components.push(other),
        }
    }
    components.iter().collect()
}

// ---------------------------------------------------------------------------
// Host function registration helper
// ---------------------------------------------------------------------------

/// Register all host functions with an Extism `PluginBuilder`.
///
/// Registers:
/// - 14 host functions in the `extism:host/user` namespace (`astrid_*`)
/// - 3 shim functions in the `shim` namespace (for `QuickJS` kernel dispatch)
#[allow(clippy::too_many_lines)]
pub fn register_host_functions(
    builder: extism::PluginBuilder,
    user_data: UserData<HostState>,
) -> extism::PluginBuilder {
    use extism::ValType;

    builder
        // ── extism:host/user namespace (standard host functions) ──
        // Registered alphabetically to match shim dispatch indices.
        .with_function(
            "astrid_channel_send",
            [PTR, PTR, PTR],
            [PTR],
            user_data.clone(),
            astrid_channel_send_impl,
        )
        .with_function(
            "astrid_fs_exists",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_fs_exists_impl,
        )
        .with_function(
            "astrid_fs_mkdir",
            [PTR],
            [],
            user_data.clone(),
            astrid_fs_mkdir_impl,
        )
        .with_function(
            "astrid_fs_readdir",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_fs_readdir_impl,
        )
        .with_function(
            "astrid_fs_stat",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_fs_stat_impl,
        )
        .with_function(
            "astrid_fs_unlink",
            [PTR],
            [],
            user_data.clone(),
            astrid_fs_unlink_impl,
        )
        .with_function(
            "astrid_get_config",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_get_config_impl,
        )
        .with_function(
            "astrid_http_request",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_http_request_impl,
        )
        .with_function(
            "astrid_kv_get",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_kv_get_impl,
        )
        .with_function(
            "astrid_kv_set",
            [PTR, PTR],
            [],
            user_data.clone(),
            astrid_kv_set_impl,
        )
        .with_function(
            "astrid_log",
            [PTR, PTR],
            [],
            user_data.clone(),
            astrid_log_impl,
        )
        .with_function(
            "astrid_read_file",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_read_file_impl,
        )
        .with_function(
            "astrid_register_connector",
            [PTR, PTR, PTR],
            [PTR],
            user_data.clone(),
            astrid_register_connector_impl,
        )
        .with_function(
            "astrid_write_file",
            [PTR, PTR],
            [],
            user_data.clone(),
            astrid_write_file_impl,
        )
        // ── shim namespace (QuickJS kernel dispatch layer) ──
        //
        // The QuickJS kernel imports 3 functions from the `shim` namespace to
        // handle host function type introspection and dispatch. These are
        // normally provided by a generated shim WASM merged via wasm-merge.
        // We provide them as host functions instead, eliminating the merge step.
        //
        // Host function indices (alphabetically sorted):
        //   0: astrid_channel_send        (PTR, PTR, PTR) -> PTR
        //   1: astrid_fs_exists           (PTR) -> PTR
        //   2: astrid_fs_mkdir            (PTR) -> void
        //   3: astrid_fs_readdir          (PTR) -> PTR
        //   4: astrid_fs_stat             (PTR) -> PTR
        //   5: astrid_fs_unlink           (PTR) -> void
        //   6: astrid_get_config          (PTR) -> PTR
        //   7: astrid_http_request        (PTR) -> PTR
        //   8: astrid_kv_get              (PTR) -> PTR
        //   9: astrid_kv_set              (PTR, PTR) -> void
        //  10: astrid_log                 (PTR, PTR) -> void
        //  11: astrid_read_file           (PTR) -> PTR
        //  12: astrid_register_connector  (PTR, PTR, PTR) -> PTR
        //  13: astrid_write_file          (PTR, PTR) -> void
        .with_function_in_namespace(
            "shim",
            "__get_function_arg_type",
            [ValType::I32, ValType::I32],
            [ValType::I32],
            UserData::new(()),
            shim_get_function_arg_type,
        )
        .with_function_in_namespace(
            "shim",
            "__get_function_return_type",
            [ValType::I32],
            [ValType::I32],
            UserData::new(()),
            shim_get_function_return_type,
        )
        .with_function_in_namespace(
            "shim",
            "__invokeHostFunc",
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
            ],
            [ValType::I64],
            user_data,
            shim_invoke_host_func,
        )
}

// ---------------------------------------------------------------------------
// QuickJS shim functions (shim:: namespace)
// ---------------------------------------------------------------------------

/// Type codes used by the `QuickJS` kernel for host function dispatch.
const TYPE_VOID: i32 = 0;
const TYPE_I64: i32 = 2;

/// Number of host functions.
const NUM_HOST_FNS: i32 = 14;

/// Number of arguments per host function (alphabetically sorted).
///
/// ```text
/// [channel_send=3, fs_exists=1, fs_mkdir=1, fs_readdir=1, fs_stat=1,
///  fs_unlink=1, get_config=1, http_request=1, kv_get=1, kv_set=2,
///  log=2, read_file=1, register_connector=3, write_file=2]
/// ```
const HOST_FN_ARG_COUNTS: [i32; 14] = [3, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 1, 3, 2];

/// Return type per host function: 0=void, 2=i64.
///
/// ```text
/// [channel_send→i64, fs_exists→i64, fs_mkdir→void, fs_readdir→i64,
///  fs_stat→i64, fs_unlink→void, get_config→i64, http_request→i64,
///  kv_get→i64, kv_set→void, log→void, read_file→i64,
///  register_connector→i64, write_file→void]
/// ```
const HOST_FN_RETURN_TYPES: [i32; 14] = [
    TYPE_I64, TYPE_I64, TYPE_VOID, TYPE_I64, TYPE_I64, TYPE_VOID, TYPE_I64, TYPE_I64, TYPE_I64,
    TYPE_VOID, TYPE_VOID, TYPE_I64, TYPE_I64, TYPE_VOID,
];

/// `shim::__get_function_arg_type(func_idx, arg_idx) -> type_code`
///
/// Returns the WASM type code for a host function argument.
/// All our host functions use i64 (memory offset) arguments.
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn shim_get_function_arg_type(
    _plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();
    let arg_idx = inputs[1].unwrap_i32();

    #[allow(clippy::cast_sign_loss)]
    let type_code = if (0..NUM_HOST_FNS).contains(&func_idx)
        && (0..HOST_FN_ARG_COUNTS[func_idx as usize]).contains(&arg_idx)
    {
        TYPE_I64 // All our args are i64 (memory offsets)
    } else {
        TYPE_VOID
    };

    outputs[0] = Val::I32(type_code);
    Ok(())
}

/// `shim::__get_function_return_type(func_idx) -> type_code`
///
/// Returns the WASM type code for a host function's return value.
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn shim_get_function_return_type(
    _plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();

    #[allow(clippy::cast_sign_loss)]
    let type_code = if (0..NUM_HOST_FNS).contains(&func_idx) {
        HOST_FN_RETURN_TYPES[func_idx as usize]
    } else {
        TYPE_VOID
    };

    outputs[0] = Val::I32(type_code);
    Ok(())
}

/// `shim::__invokeHostFunc(func_idx, arg0, arg1, arg2, arg3, arg4) -> i64`
///
/// Dispatches a host function call from the `QuickJS` kernel.
/// Arguments are passed as i64 bit patterns (memory offsets for our functions).
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
fn shim_invoke_host_func(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();
    let args = &inputs[1..]; // arg0..arg4 as i64

    // Dispatch based on alphabetically sorted function index.
    // Each branch repackages i64 args as Val::I64 and delegates to the
    // actual host function implementation.
    match func_idx {
        0 => {
            // astrid_channel_send(PTR, PTR, PTR) -> PTR
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
                Val::I64(args[2].unwrap_i64()),
            ];
            let mut fn_outputs = [Val::I64(0)];
            astrid_channel_send_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        1 => {
            // astrid_fs_exists(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_fs_exists_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        2 => {
            // astrid_fs_mkdir(PTR) -> void
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [];
            astrid_fs_mkdir_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        3 => {
            // astrid_fs_readdir(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_fs_readdir_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        4 => {
            // astrid_fs_stat(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_fs_stat_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        5 => {
            // astrid_fs_unlink(PTR) -> void
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [];
            astrid_fs_unlink_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        6 => {
            // astrid_get_config(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_get_config_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        7 => {
            // astrid_http_request(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_http_request_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        8 => {
            // astrid_kv_get(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_kv_get_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        9 => {
            // astrid_kv_set(PTR, PTR) -> void
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
            ];
            let mut fn_outputs = [];
            astrid_kv_set_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        10 => {
            // astrid_log(PTR, PTR) -> void
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
            ];
            let mut fn_outputs = [];
            astrid_log_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        11 => {
            // astrid_read_file(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_read_file_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        12 => {
            // astrid_register_connector(PTR, PTR, PTR) -> PTR
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
                Val::I64(args[2].unwrap_i64()),
            ];
            let mut fn_outputs = [Val::I64(0)];
            astrid_register_connector_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        13 => {
            // astrid_write_file(PTR, PTR) -> void
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
            ];
            let mut fn_outputs = [];
            astrid_write_file_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        _ => {
            outputs[0] = Val::I64(0);
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_boundary_relative_within() {
        let root = std::env::temp_dir();
        let result = resolve_within_workspace(&root, "subdir/file.txt");
        assert!(result.is_ok());
    }

    #[test]
    fn workspace_boundary_traversal_rejected() {
        let root = std::env::temp_dir().join("fake-workspace");
        let _ = std::fs::create_dir_all(&root);
        let result = resolve_within_workspace(&root, "../../etc/passwd");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("escapes workspace boundary"), "got: {err}");
    }

    #[test]
    fn lexical_normalize_removes_dotdot() {
        let normalized = lexical_normalize(Path::new("/a/b/../c/./d"));
        assert_eq!(normalized, Path::new("/a/c/d"));
    }

    #[test]
    fn lexical_normalize_handles_only_dots() {
        let normalized = lexical_normalize(Path::new("./foo/./bar"));
        assert_eq!(normalized, Path::new("foo/bar"));
    }

    /// Verify host function metadata is in strict alphabetical order.
    ///
    /// The shim dispatch layer, `HOST_FN_ARG_COUNTS`, `HOST_FN_RETURN_TYPES`,
    /// and the `shim_invoke_host_func` match arms ALL depend on alphabetical
    /// ordering. This test catches any desynchronization.
    #[test]
    fn host_function_ordering_is_alphabetical() {
        // Canonical alphabetically sorted host function names.
        // This list is the single source of truth — if a function is added,
        // it must be inserted here in sorted order.
        let expected_order = [
            "astrid_channel_send",
            "astrid_fs_exists",
            "astrid_fs_mkdir",
            "astrid_fs_readdir",
            "astrid_fs_stat",
            "astrid_fs_unlink",
            "astrid_get_config",
            "astrid_http_request",
            "astrid_kv_get",
            "astrid_kv_set",
            "astrid_log",
            "astrid_read_file",
            "astrid_register_connector",
            "astrid_write_file",
        ];

        // Verify count matches constants
        assert_eq!(
            expected_order.len() as i32,
            NUM_HOST_FNS,
            "NUM_HOST_FNS doesn't match expected function count"
        );
        assert_eq!(
            HOST_FN_ARG_COUNTS.len(),
            expected_order.len(),
            "HOST_FN_ARG_COUNTS length mismatch"
        );
        assert_eq!(
            HOST_FN_RETURN_TYPES.len(),
            expected_order.len(),
            "HOST_FN_RETURN_TYPES length mismatch"
        );

        // Verify the list is actually sorted
        let mut sorted = expected_order;
        sorted.sort();
        assert_eq!(
            expected_order, sorted,
            "host function names must be alphabetically sorted"
        );

        // Verify arg counts match expected signatures:
        //   channel_send(connector_id,user_id,content)=3,
        //   fs_exists(path)=1, fs_mkdir(path)=1, fs_readdir(path)=1,
        //   fs_stat(path)=1, fs_unlink(path)=1,
        //   get_config(key)=1, http_request(json)=1, kv_get(key)=1,
        //   kv_set(key,val)=2, log(level,msg)=2, read_file(path)=1,
        //   register_connector(name,platform,profile)=3, write_file(path,content)=2
        let expected_args = [3, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 1, 3, 2];
        assert_eq!(
            HOST_FN_ARG_COUNTS, expected_args,
            "HOST_FN_ARG_COUNTS doesn't match expected signatures"
        );

        // Verify return types match:
        //   channel_send→i64, fs_exists→i64, fs_mkdir→void, fs_readdir→i64,
        //   fs_stat→i64, fs_unlink→void, get_config→i64, http_request→i64,
        //   kv_get→i64, kv_set→void, log→void, read_file→i64,
        //   register_connector→i64, write_file→void
        let expected_returns = [
            TYPE_I64, TYPE_I64, TYPE_VOID, TYPE_I64, TYPE_I64, TYPE_VOID, TYPE_I64, TYPE_I64,
            TYPE_I64, TYPE_VOID, TYPE_VOID, TYPE_I64, TYPE_I64, TYPE_VOID,
        ];
        assert_eq!(
            HOST_FN_RETURN_TYPES, expected_returns,
            "HOST_FN_RETURN_TYPES doesn't match expected signatures"
        );
    }

    #[test]
    fn parse_known_frontend_types() {
        use astrid_core::identity::FrontendType;

        assert_eq!(parse_frontend_type("discord"), FrontendType::Discord);
        assert_eq!(parse_frontend_type("Discord"), FrontendType::Discord);
        assert_eq!(parse_frontend_type("whatsapp"), FrontendType::WhatsApp);
        assert_eq!(parse_frontend_type("whats_app"), FrontendType::WhatsApp);
        assert_eq!(parse_frontend_type("telegram"), FrontendType::Telegram);
        assert_eq!(parse_frontend_type("slack"), FrontendType::Slack);
        assert_eq!(parse_frontend_type("web"), FrontendType::Web);
        assert_eq!(parse_frontend_type("cli"), FrontendType::Cli);
    }

    #[test]
    fn parse_unknown_frontend_type_becomes_custom() {
        use astrid_core::identity::FrontendType;

        assert_eq!(
            parse_frontend_type("matrix"),
            FrontendType::Custom("matrix".into())
        );
    }

    #[test]
    fn parse_valid_connector_profiles() {
        use astrid_core::ConnectorProfile;

        assert_eq!(
            parse_connector_profile("chat").unwrap(),
            ConnectorProfile::Chat
        );
        assert_eq!(
            parse_connector_profile("interactive").unwrap(),
            ConnectorProfile::Interactive
        );
        assert_eq!(
            parse_connector_profile("notify").unwrap(),
            ConnectorProfile::Notify
        );
        assert_eq!(
            parse_connector_profile("bridge").unwrap(),
            ConnectorProfile::Bridge
        );
        assert_eq!(
            parse_connector_profile("Chat").unwrap(),
            ConnectorProfile::Chat
        );
    }

    #[test]
    fn parse_invalid_connector_profile_rejected() {
        assert!(parse_connector_profile("unknown").is_err());
        assert!(parse_connector_profile("").is_err());
    }

    #[test]
    fn max_string_length_constant_is_reasonable() {
        // Sanity check: MAX_STRING_LENGTH shouldn't be 0 or absurdly large
        assert!(MAX_STRING_LENGTH >= 64, "MAX_STRING_LENGTH too small");
        assert!(MAX_STRING_LENGTH <= 4096, "MAX_STRING_LENGTH too large");
    }

    #[test]
    fn max_inbound_message_bytes_constant_is_one_mb() {
        assert_eq!(MAX_INBOUND_MESSAGE_BYTES, 1_048_576);
    }

    #[test]
    fn parse_frontend_type_with_whitespace_matches_known() {
        use astrid_core::identity::FrontendType;

        // After trimming at the call site, whitespace-padded known platforms
        // should resolve to the correct variant.
        assert_eq!(parse_frontend_type("telegram"), FrontendType::Telegram);
        // Pre-trimmed input — caller is responsible for trimming.
        assert_eq!(
            parse_frontend_type("  telegram  "),
            FrontendType::Custom("  telegram  ".into())
        );
    }

    #[test]
    fn parse_frontend_type_unknown_is_lowercased() {
        use astrid_core::identity::FrontendType;

        // Unknown platforms are lowercased by to_lowercase() in the match.
        assert_eq!(
            parse_frontend_type("MATRIX"),
            FrontendType::Custom("matrix".into())
        );
    }
}
