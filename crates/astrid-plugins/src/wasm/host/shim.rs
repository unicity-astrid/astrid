use super::WasmHostFunction;
use crate::wasm::host_state::HostState;
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

    #[allow(clippy::cast_sign_loss)]
    let type_code = if let Some(func) = WasmHostFunction::from_index(func_idx as usize) {
        if (0..func.arg_count()).contains(&arg_idx) {
            TYPE_I64
        } else {
            TYPE_VOID
        }
    } else {
        TYPE_VOID
    };

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

    #[allow(clippy::cast_sign_loss)]
    let type_code = if let Some(func) = WasmHostFunction::from_index(func_idx as usize) {
        func.return_type()
    } else {
        TYPE_VOID
    };

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

    let Some(func) = WasmHostFunction::from_index(func_idx as usize) else {
        outputs[0] = Val::I64(0);
        return Ok(());
    };

    let arg_count = func.arg_count() as usize;
    let mut fn_inputs = Vec::with_capacity(arg_count);
    for i in 0..arg_count {
        fn_inputs.push(Val::I64(args[i].unwrap_i64()));
    }

    let mut fn_outputs = if func.return_type() == TYPE_VOID {
        vec![]
    } else {
        vec![Val::I64(0)]
    };

    match func {
        WasmHostFunction::ChannelSend => crate::wasm::host::channel::astrid_channel_send_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsExists => crate::wasm::host::fs::astrid_fs_exists_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsMkdir => crate::wasm::host::fs::astrid_fs_mkdir_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsReaddir => crate::wasm::host::fs::astrid_fs_readdir_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsStat => crate::wasm::host::fs::astrid_fs_stat_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::FsUnlink => crate::wasm::host::fs::astrid_fs_unlink_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::GetConfig => crate::wasm::host::sys::astrid_get_config_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::HttpRequest => crate::wasm::host::http::astrid_http_request_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::KvGet => crate::wasm::host::kv::astrid_kv_get_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::KvSet => crate::wasm::host::kv::astrid_kv_set_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::Log => {
            crate::wasm::host::sys::astrid_log_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?
        },
        WasmHostFunction::ReadFile => crate::wasm::host::fs::astrid_read_file_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
        WasmHostFunction::RegisterConnector => {
            crate::wasm::host::channel::astrid_register_connector_impl(
                plugin,
                &fn_inputs,
                &mut fn_outputs,
                user_data,
            )?
        },
        WasmHostFunction::WriteFile => crate::wasm::host::fs::astrid_write_file_impl(
            plugin,
            &fn_inputs,
            &mut fn_outputs,
            user_data,
        )?,
    }

    if !fn_outputs.is_empty() {
        outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
    } else {
        outputs[0] = Val::I64(0);
    }

    Ok(())
}
