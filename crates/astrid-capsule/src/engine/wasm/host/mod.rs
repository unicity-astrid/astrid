/// Cron scheduling.
pub mod cron;
/// File system operations for plugins.
pub mod fs;
/// HTTP network executions for plugins.
pub mod http;
/// Inter-Process Communication bus.
pub mod ipc;
/// Key-Value persistent storage primitives.
pub mod kv;
/// `QuickJS` ABI definitions.
pub mod shim;
/// System configuration primitives.
pub mod sys;
/// Uplink communications with host capabilities.
pub mod uplink;
/// Utility functions for WASM host implementations.
pub mod util;

use crate::engine::wasm::host_state::HostState;
use extism::{PluginBuilder, UserData, ValType};

/// Registry of explicitly supported capability functions exposed to the WASM Runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmHostFunction {
    FsExists,
    FsMkdir,
    FsReaddir,
    FsStat,
    FsUnlink,
    ReadFile,
    WriteFile,
    IpcPublish,
    IpcSubscribe,
    IpcUnsubscribe,
    IpcPoll,
    UplinkRegister,
    UplinkSend,
    UplinkReceive,
    KvGet,
    KvSet,
    GetConfig,
    HttpRequest,
    Log,
    CronSchedule,
    CronCancel,
}

impl WasmHostFunction {
    pub const ALL: [Self; 21] = [
        Self::FsExists,
        Self::FsMkdir,
        Self::FsReaddir,
        Self::FsStat,
        Self::FsUnlink,
        Self::ReadFile,
        Self::WriteFile,
        Self::IpcPublish,
        Self::IpcSubscribe,
        Self::IpcUnsubscribe,
        Self::IpcPoll,
        Self::UplinkRegister,
        Self::UplinkSend,
        Self::UplinkReceive,
        Self::KvGet,
        Self::KvSet,
        Self::GetConfig,
        Self::HttpRequest,
        Self::Log,
        Self::CronSchedule,
        Self::CronCancel,
    ];

    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::FsExists => "astrid_fs_exists",
            Self::FsMkdir => "astrid_fs_mkdir",
            Self::FsReaddir => "astrid_fs_readdir",
            Self::FsStat => "astrid_fs_stat",
            Self::FsUnlink => "astrid_fs_unlink",
            Self::ReadFile => "astrid_read_file",
            Self::WriteFile => "astrid_write_file",
            Self::IpcPublish => "astrid_ipc_publish",
            Self::IpcSubscribe => "astrid_ipc_subscribe",
            Self::IpcUnsubscribe => "astrid_ipc_unsubscribe",
            Self::IpcPoll => "astrid_ipc_poll",
            Self::UplinkRegister => "astrid_uplink_register",
            Self::UplinkSend => "astrid_uplink_send",
            Self::UplinkReceive => "astrid_uplink_receive",
            Self::KvGet => "astrid_kv_get",
            Self::KvSet => "astrid_kv_set",
            Self::GetConfig => "astrid_get_config",
            Self::HttpRequest => "astrid_http_request",
            Self::Log => "astrid_log",
            Self::CronSchedule => "astrid_cron_schedule",
            Self::CronCancel => "astrid_cron_cancel",
        }
    }

    #[must_use]
    pub fn arg_count(self) -> usize {
        match self {
            Self::FsExists
            | Self::FsMkdir
            | Self::FsReaddir
            | Self::FsStat
            | Self::FsUnlink
            | Self::ReadFile
            | Self::IpcSubscribe
            | Self::IpcUnsubscribe
            | Self::IpcPoll
            | Self::UplinkReceive
            | Self::KvGet
            | Self::GetConfig
            | Self::HttpRequest
            | Self::CronCancel => 1,
            Self::WriteFile | Self::IpcPublish | Self::KvSet | Self::Log => 2,
            Self::UplinkRegister | Self::UplinkSend | Self::CronSchedule => 3,
        }
    }

    #[must_use]
    pub fn return_type(self) -> i32 {
        use shim::{TYPE_I64, TYPE_VOID};
        match self {
            Self::FsMkdir
            | Self::FsUnlink
            | Self::WriteFile
            | Self::IpcPublish
            | Self::IpcUnsubscribe
            | Self::KvSet
            | Self::Log
            | Self::CronSchedule
            | Self::CronCancel => TYPE_VOID,
            Self::FsExists
            | Self::FsReaddir
            | Self::FsStat
            | Self::ReadFile
            | Self::IpcSubscribe
            | Self::IpcPoll
            | Self::UplinkRegister
            | Self::UplinkSend
            | Self::UplinkReceive
            | Self::KvGet
            | Self::GetConfig
            | Self::HttpRequest => TYPE_I64,
        }
    }
}

pub fn register_host_functions(
    mut builder: PluginBuilder,
    user_data: UserData<HostState>,
) -> PluginBuilder {
    for func in WasmHostFunction::ALL {
        let ud = user_data.clone();

        let args = vec![extism::PTR; func.arg_count()];
        let rets = if func.return_type() == shim::TYPE_I64 {
            vec![extism::PTR]
        } else {
            vec![]
        };

        builder = match func {
            WasmHostFunction::FsExists => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_fs_exists_impl)
            },
            WasmHostFunction::FsMkdir => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_fs_mkdir_impl)
            },
            WasmHostFunction::FsReaddir => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_fs_readdir_impl)
            },
            WasmHostFunction::FsStat => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_fs_stat_impl)
            },
            WasmHostFunction::FsUnlink => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_fs_unlink_impl)
            },
            WasmHostFunction::ReadFile => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_read_file_impl)
            },
            WasmHostFunction::WriteFile => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_write_file_impl)
            },
            WasmHostFunction::IpcPublish => {
                builder.with_function(func.name(), args, rets, ud, ipc::astrid_ipc_publish_impl)
            },
            WasmHostFunction::IpcSubscribe => {
                builder.with_function(func.name(), args, rets, ud, ipc::astrid_ipc_subscribe_impl)
            },
            WasmHostFunction::IpcUnsubscribe => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                ipc::astrid_ipc_unsubscribe_impl,
            ),
            WasmHostFunction::IpcPoll => {
                builder.with_function(func.name(), args, rets, ud, ipc::astrid_ipc_poll_impl)
            },
            WasmHostFunction::UplinkRegister => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                uplink::astrid_uplink_register_impl,
            ),
            WasmHostFunction::UplinkSend => {
                builder.with_function(func.name(), args, rets, ud, uplink::astrid_uplink_send_impl)
            },
            WasmHostFunction::UplinkReceive => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                uplink::astrid_uplink_receive_impl,
            ),
            WasmHostFunction::KvGet => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_get_impl)
            },
            WasmHostFunction::KvSet => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_set_impl)
            },
            WasmHostFunction::GetConfig => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_get_config_impl)
            },
            WasmHostFunction::HttpRequest => {
                builder.with_function(func.name(), args, rets, ud, http::astrid_http_request_impl)
            },
            WasmHostFunction::Log => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_log_impl)
            },
            WasmHostFunction::CronSchedule => {
                builder.with_function(func.name(), args, rets, ud, cron::astrid_cron_schedule_impl)
            },
            WasmHostFunction::CronCancel => {
                builder.with_function(func.name(), args, rets, ud, cron::astrid_cron_cancel_impl)
            },
        };
    }

    builder
        .with_function_in_namespace(
            "shim",
            "__get_function_arg_type",
            [ValType::I32, ValType::I32],
            [ValType::I32],
            UserData::new(()),
            shim::shim_get_function_arg_type,
        )
        .with_function_in_namespace(
            "shim",
            "__get_function_return_type",
            [ValType::I32],
            [ValType::I32],
            UserData::new(()),
            shim::shim_get_function_return_type,
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
            shim::shim_invoke_host_func,
        )
}
