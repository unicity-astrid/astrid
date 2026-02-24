use super::WasmHostFunction;
use crate::engine::wasm::host_state::HostState;
use extism::{CurrentPlugin, Error, UserData, Val};

pub(crate) const TYPE_VOID: i32 = 0;
pub(crate) const TYPE_I64: i32 = 2;

#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
pub(crate) fn shim_get_function_arg_type(
    _plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();
    let arg_idx = inputs[1].unwrap_i32();

    let type_code = usize::try_from(func_idx)
        .ok()
        .and_then(WasmHostFunction::from_index)
        .map_or(TYPE_VOID, |func| {
            if usize::try_from(arg_idx).is_ok_and(|idx| idx < func.arg_count()) {
                TYPE_I64
            } else {
                TYPE_VOID
            }
        });

    outputs[0] = Val::I32(type_code);
    Ok(())
}

#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
pub(crate) fn shim_get_function_return_type(
    _plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();

    let type_code = usize::try_from(func_idx)
        .ok()
        .and_then(WasmHostFunction::from_index)
        .map_or(TYPE_VOID, WasmHostFunction::return_type);

    outputs[0] = Val::I32(type_code);
    Ok(())
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub(crate) fn shim_invoke_host_func(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();
    let args = &inputs[1..];

    let Some(func) = usize::try_from(func_idx)
        .ok()
        .and_then(WasmHostFunction::from_index)
    else {
        outputs[0] = Val::I64(0);
        return Ok(());
    };

    let arg_count = func.arg_count();
    let mut fn_inputs = Vec::with_capacity(arg_count);
    for arg in args.iter().take(arg_count) {
        fn_inputs.push(Val::I64(arg.unwrap_i64()));
    }

    let mut fn_outputs = if func.return_type() == TYPE_VOID {
        vec![]
    } else {
        vec![Val::I64(0)]
    };

    match func {
        WasmHostFunction::FsExists => crate::engine::wasm::host::fs::astrid_fs_exists_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsMkdir => crate::engine::wasm::host::fs::astrid_fs_mkdir_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsReaddir => crate::engine::wasm::host::fs::astrid_fs_readdir_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsStat => crate::engine::wasm::host::fs::astrid_fs_stat_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsUnlink => crate::engine::wasm::host::fs::astrid_fs_unlink_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::GetConfig => crate::engine::wasm::host::sys::astrid_get_config_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::HttpRequest => crate::engine::wasm::host::http::astrid_http_request_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::IpcPublish => crate::engine::wasm::host::ipc::astrid_ipc_publish_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::IpcSubscribe => {
            crate::engine::wasm::host::ipc::astrid_ipc_subscribe_impl(
                plugin,
                &fn_inputs,
                &mut fn_outputs,
                user_data,
            )?
        },
        WasmHostFunction::IpcUnsubscribe => {
            crate::engine::wasm::host::ipc::astrid_ipc_unsubscribe_impl(
                plugin,
                &fn_inputs,
                &mut fn_outputs,
                user_data,
            )?
        },
        WasmHostFunction::IpcPoll => crate::engine::wasm::host::ipc::astrid_ipc_poll_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::KvGet => crate::engine::wasm::host::kv::astrid_kv_get_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::KvSet => crate::engine::wasm::host::kv::astrid_kv_set_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::Log => {
            crate::engine::wasm::host::sys::astrid_log_impl(
                plugin,
                &fn_inputs,
                &mut fn_outputs,
                user_data,
            )?;
        },
        WasmHostFunction::ReadFile => crate::engine::wasm::host::fs::astrid_read_file_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::UplinkRegister => {
            crate::engine::wasm::host::uplink::astrid_uplink_register_impl(
                plugin,
                &fn_inputs,
                &mut fn_outputs,
                user_data,
            )?;
        },
        WasmHostFunction::UplinkSend => {
            crate::engine::wasm::host::uplink::astrid_uplink_send_impl(
                plugin,
                &fn_inputs,
                &mut fn_outputs,
                user_data,
            )?;
        },

        WasmHostFunction::WriteFile => crate::engine::wasm::host::fs::astrid_write_file_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::CronSchedule => {
            crate::engine::wasm::host::cron::astrid_cron_schedule_impl(
                plugin,
                &fn_inputs,
                &mut fn_outputs,
                user_data,
            )?
        },
        WasmHostFunction::CronCancel => crate::engine::wasm::host::cron::astrid_cron_cancel_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
    }

    if fn_outputs.is_empty() {
        outputs[0] = Val::I64(0);
    } else {
        outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
    }

    Ok(())
}
