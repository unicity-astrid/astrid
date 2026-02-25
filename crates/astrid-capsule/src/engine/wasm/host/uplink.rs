use astrid_core::ConnectorProfile;
use astrid_core::identity::FrontendType;
use extism::{CurrentPlugin, Error, UserData, Val};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

pub(crate) const MAX_INBOUND_MESSAGE_BYTES: usize = 1_048_576;

pub(crate) fn parse_frontend_type(platform: &[u8]) -> FrontendType {
    let platform_str = String::from_utf8_lossy(platform).to_lowercase();
    match platform_str.as_str() {
        "discord" => FrontendType::Discord,
        "whatsapp" | "whats_app" => FrontendType::WhatsApp,
        "telegram" => FrontendType::Telegram,
        "slack" => FrontendType::Slack,
        "web" => FrontendType::Web,
        "cli" => FrontendType::Cli,
        other => FrontendType::Custom(other.to_string()),
    }
}

pub(crate) fn parse_connector_profile(profile: &[u8]) -> Result<ConnectorProfile, Error> {
    let profile_str = String::from_utf8_lossy(profile).to_lowercase();
    match profile_str.as_str() {
        "chat" => Ok(ConnectorProfile::Chat),
        "interactive" => Ok(ConnectorProfile::Interactive),
        "notify" => Ok(ConnectorProfile::Notify),
        "bridge" => Ok(ConnectorProfile::Bridge),
        "human" => Ok(ConnectorProfile::Chat), // Fallback map
        other => Err(Error::msg(format!(
            "invalid uplink profile: {other:?} (expected: chat, interactive, notify, bridge, human)"
        ))),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_uplink_send_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let uplink_id_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], 128)?;
    let platform_user_id: String =
        String::from_utf8(util::get_safe_bytes(plugin, &inputs[1], 512)?).unwrap_or_default();
    let content: String = String::from_utf8(util::get_safe_bytes(
        plugin,
        &inputs[2],
        MAX_INBOUND_MESSAGE_BYTES as u64,
    )?)
    .unwrap_or_default();

    if uplink_id_bytes.len() > 64 {
        return Err(Error::msg("uplink_id too long"));
    }

    let connector_uuid: uuid::Uuid = String::from_utf8_lossy(&uplink_id_bytes)
        .parse()
        .map_err(|e| Error::msg(format!("invalid uplink_id: {e}")))?;
    let connector_id = astrid_core::ConnectorId::from_uuid(connector_uuid);

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let capsule_id = state.capsule_id.as_str().to_owned();
    let inbound_tx = state.inbound_tx.clone();

    // In a full implementation we would check the manifest for Uplink definitions
    // For now we assume they have the capability.

    let platform = state
        .registered_connectors
        .iter()
        .find(|c| c.id == connector_id)
        .map(|c| c.frontend_type.clone())
        .ok_or_else(|| {
            Error::msg(format!(
                "uplink {connector_id} not registered by capsule {capsule_id}"
            ))
        })?;
    drop(state);

    let tx = inbound_tx
        .ok_or_else(|| Error::msg(format!("capsule {capsule_id} has no inbound channel")))?;

    let message =
        astrid_core::InboundMessage::builder(connector_id, platform, platform_user_id, content)
            .build();

    let result = match tx.try_send(message) {
        Ok(()) => serde_json::json!({"ok": true}),
        Err(_) => {
            serde_json::json!({"ok": false, "dropped": true})
        },
    };

    let result = result.to_string();
    let mem = plugin.memory_new(&result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_uplink_register_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let name: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], 512)?;
    let platform_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[1], 512)?;
    let profile_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[2], 512)?;

    let name_str = String::from_utf8_lossy(&name).into_owned();

    let frontend_type = parse_frontend_type(&platform_bytes);
    let profile = parse_connector_profile(&profile_bytes)?;

    let ud = user_data.get()?;
    let (capsule_id, security, handle) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        (
            state.capsule_id.as_str().to_owned(),
            state.security.clone(),
            state.runtime_handle.clone(),
        )
    };

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = capsule_id.clone();
        let cname = name_str.clone();
        let plat = String::from_utf8_lossy(&platform_bytes).into_owned();
        let check = tokio::task::block_in_place(|| {
            handle.block_on(async move { gate.check_connector_register(&pid, &cname, &plat).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied uplink registration: {reason}"
            )));
        }
    }

    let source = astrid_core::ConnectorSource::new_wasm(&capsule_id).map_err(|e| {
        Error::msg(format!(
            "failed to create uplink source for capsule {capsule_id}: {e}"
        ))
    })?;

    let descriptor = astrid_core::ConnectorDescriptor::builder(name_str, frontend_type)
        .source(source)
        .capabilities(astrid_core::ConnectorCapabilities::receive_only())
        .profile(profile)
        .build();

    let connector_id = descriptor.id.to_string();

    {
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        state
            .register_connector(descriptor)
            .map_err(|e| Error::msg(format!("capsule {capsule_id}: {e}")))?;
    }

    let mem = plugin.memory_new(&connector_id)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}
