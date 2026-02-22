use astrid_core::ConnectorProfile;
use astrid_core::identity::FrontendType;
use extism::{CurrentPlugin, Error, UserData, Val};

use crate::wasm::host::util;
use crate::wasm::host_state::HostState;

pub(crate) const MAX_INBOUND_MESSAGE_BYTES: usize = 1_048_576;
pub(crate) const MAX_STRING_LENGTH: usize = 256;

#[allow(dead_code)]
pub(crate) fn parse_frontend_type(platform: &str) -> FrontendType {
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

#[allow(dead_code)]
pub(crate) fn parse_connector_profile(profile: &str) -> Result<ConnectorProfile, Error> {
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

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_channel_send_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let connector_id_str: String = util::get_safe_string(plugin, &inputs[0], 128)?;
    let platform_user_id: String = util::get_safe_string(plugin, &inputs[1], 512)?;
    let content: String =
        util::get_safe_string(plugin, &inputs[2], MAX_INBOUND_MESSAGE_BYTES as u64)?;

    if connector_id_str.len() > 64 {
        return Err(Error::msg("connector_id too long"));
    }

    if platform_user_id.len() > MAX_STRING_LENGTH {
        return Err(Error::msg(format!(
            "platform_user_id too long: {} bytes (max {MAX_STRING_LENGTH})",
            platform_user_id.len()
        )));
    }

    if content.len() > MAX_INBOUND_MESSAGE_BYTES {
        return Err(Error::msg(format!(
            "inbound message content too large: {} bytes (max {})",
            content.len(),
            MAX_INBOUND_MESSAGE_BYTES
        )));
    }

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

    if !state.has_connector_capability {
        return Err(Error::msg(format!(
            "plugin {plugin_id} does not declare Connector capability"
        )));
    }

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

    let tx = inbound_tx.ok_or_else(|| {
        Error::msg(format!(
            "plugin {plugin_id} has no inbound channel — is Connector capability declared?"
        ))
    })?;

    let message =
        astrid_core::InboundMessage::builder(connector_id, platform, platform_user_id, content)
            .build();

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

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_register_connector_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let name: String = util::get_safe_string(plugin, &inputs[0], 512)?;
    let platform_str: String = util::get_safe_string(plugin, &inputs[1], 512)?;
    let profile_str: String = util::get_safe_string(plugin, &inputs[2], 512)?;

    if name.is_empty() {
        return Err(Error::msg("connector name must not be empty"));
    }
    if name.len() > MAX_STRING_LENGTH {
        return Err(Error::msg(format!(
            "connector name too long: {} bytes (max {MAX_STRING_LENGTH})",
            name.len()
        )));
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

    let frontend_type = parse_frontend_type(&platform_str);
    let profile = parse_connector_profile(&profile_str)?;

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

    if !has_capability {
        return Err(Error::msg(format!(
            "plugin {plugin_id} does not declare Connector capability"
        )));
    }

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

    let source = astrid_core::ConnectorSource::new_wasm(&plugin_id).map_err(|e| {
        Error::msg(format!(
            "failed to create connector source for plugin {plugin_id}: {e}"
        ))
    })?;

    let descriptor = astrid_core::ConnectorDescriptor::builder(name, frontend_type)
        .source(source)
        .capabilities(astrid_core::ConnectorCapabilities::receive_only())
        .profile(profile)
        .build();

    let connector_id = descriptor.id.to_string();

    {
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        state.register_connector(descriptor).map_err(|e| {
            Error::msg(format!(
                "plugin {plugin_id}: {e} (max {})",
                astrid_core::MAX_CONNECTORS_PER_PLUGIN
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
