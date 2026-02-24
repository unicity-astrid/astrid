use extism::{CurrentPlugin, Error, UserData, Val};

use crate::engine::wasm::host_state::HostState;

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_cron_schedule_impl(
    _plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    _outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    // TODO: Forward to central OS cron scheduler in Phase 7
    tracing::warn!("Cron scheduling is not yet implemented (Phase 7)");
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_cron_cancel_impl(
    _plugin: &mut CurrentPlugin,
    _inputs: &[Val],
    _outputs: &mut [Val],
    _user_data: UserData<HostState>,
) -> Result<(), Error> {
    // TODO: Forward to central OS cron scheduler in Phase 7
    tracing::warn!("Cron scheduling is not yet implemented (Phase 7)");
    Ok(())
}
