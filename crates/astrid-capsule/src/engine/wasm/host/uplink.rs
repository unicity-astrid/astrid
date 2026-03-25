use astrid_core::UplinkProfile;

use crate::engine::wasm::bindings::astrid::capsule::uplink;
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

pub(crate) fn parse_uplink_profile(profile: &str) -> Result<UplinkProfile, String> {
    let profile_str = profile.to_lowercase();
    match profile_str.as_str() {
        "chat" => Ok(UplinkProfile::Chat),
        "interactive" => Ok(UplinkProfile::Interactive),
        "notify" => Ok(UplinkProfile::Notify),
        "bridge" => Ok(UplinkProfile::Bridge),
        "human" => Ok(UplinkProfile::Chat), // Fallback map
        other => Err(format!(
            "invalid uplink profile: {other:?} (expected: chat, interactive, notify, bridge, human)"
        )),
    }
}

impl uplink::Host for HostState {
    fn uplink_register(
        &mut self,
        name: String,
        platform: String,
        profile: String,
    ) -> Result<String, String> {
        let platform = platform.trim().to_ascii_lowercase();
        let profile = parse_uplink_profile(&profile)?;

        let capsule_id = self.capsule_id.as_str().to_owned();
        let security = self.security.clone();
        let handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        if let Some(gate) = &security {
            let gate = gate.clone();
            let pid = capsule_id.clone();
            let cname = name.clone();
            let plat = platform.clone();
            let check = util::bounded_block_on(&handle, &host_semaphore, async move {
                gate.check_uplink_register(&pid, &cname, &plat).await
            });
            if let Err(reason) = check {
                return Err(format!("security denied uplink registration: {reason}"));
            }
        }

        let source = astrid_core::UplinkSource::new_wasm(&capsule_id)
            .map_err(|e| format!("failed to create uplink source for capsule {capsule_id}: {e}"))?;

        let descriptor = astrid_core::UplinkDescriptor::builder(name, platform)
            .source(source)
            .capabilities(astrid_core::UplinkCapabilities::receive_only())
            .profile(profile)
            .build();

        let uplink_id = descriptor.id.to_string();

        self.register_uplink(descriptor)
            .map_err(|e| format!("capsule {capsule_id}: {e}"))?;

        Ok(uplink_id)
    }

    fn uplink_send(
        &mut self,
        uplink_id: String,
        platform_user_id: String,
        content: String,
    ) -> Result<bool, String> {
        if uplink_id.len() > 64 {
            return Err("uplink_id too long".to_string());
        }

        let uplink_uuid: uuid::Uuid = uplink_id
            .parse()
            .map_err(|e| format!("invalid uplink_id: {e}"))?;
        let uplink_id = astrid_core::UplinkId::from_uuid(uplink_uuid);

        let capsule_id = self.capsule_id.as_str().to_owned();
        let inbound_tx = self.inbound_tx.clone();

        let platform = self
            .registered_uplinks
            .iter()
            .find(|c| c.id == uplink_id)
            .map(|c| c.platform.clone())
            .ok_or_else(|| format!("uplink {uplink_id} not registered by capsule {capsule_id}"))?;

        let tx =
            inbound_tx.ok_or_else(|| format!("capsule {capsule_id} has no inbound channel"))?;

        let message =
            astrid_core::InboundMessage::builder(uplink_id, platform, platform_user_id, content)
                .build();

        match tx.try_send(message) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}
