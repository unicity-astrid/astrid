pub mod channel;
pub mod fs;
pub mod http;
pub mod kv;
pub mod shim;
pub mod sys;

use crate::wasm::host_state::HostState;
use extism::{PluginBuilder, UserData, ValType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmHostFunction {
    ChannelSend,
    FsExists,
    FsMkdir,
    FsReaddir,
    FsStat,
    FsUnlink,
    GetConfig,
    HttpRequest,
    KvGet,
    KvSet,
    Log,
    ReadFile,
    RegisterConnector,
    WriteFile,
}

impl WasmHostFunction {
    /// Ordered precisely as expected by the QuickJS shim kernel.
    pub const ALL: [Self; 14] = [
        Self::ChannelSend,
        Self::FsExists,
        Self::FsMkdir,
        Self::FsReaddir,
        Self::FsStat,
        Self::FsUnlink,
        Self::GetConfig,
        Self::HttpRequest,
        Self::KvGet,
        Self::KvSet,
        Self::Log,
        Self::ReadFile,
        Self::RegisterConnector,
        Self::WriteFile,
    ];

    pub fn from_index(idx: usize) -> Option<Self> {
        Self::ALL.get(idx).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ChannelSend => "astrid_channel_send",
            Self::FsExists => "astrid_fs_exists",
            Self::FsMkdir => "astrid_fs_mkdir",
            Self::FsReaddir => "astrid_fs_readdir",
            Self::FsStat => "astrid_fs_stat",
            Self::FsUnlink => "astrid_fs_unlink",
            Self::GetConfig => "astrid_get_config",
            Self::HttpRequest => "astrid_http_request",
            Self::KvGet => "astrid_kv_get",
            Self::KvSet => "astrid_kv_set",
            Self::Log => "astrid_log",
            Self::ReadFile => "astrid_read_file",
            Self::RegisterConnector => "astrid_register_connector",
            Self::WriteFile => "astrid_write_file",
        }
    }

    pub fn arg_count(self) -> i32 {
        match self {
            Self::ChannelSend => 3,
            Self::FsExists => 1,
            Self::FsMkdir => 1,
            Self::FsReaddir => 1,
            Self::FsStat => 1,
            Self::FsUnlink => 1,
            Self::GetConfig => 1,
            Self::HttpRequest => 1,
            Self::KvGet => 1,
            Self::KvSet => 2,
            Self::Log => 2,
            Self::ReadFile => 1,
            Self::RegisterConnector => 3,
            Self::WriteFile => 2,
        }
    }

    pub fn return_type(self) -> i32 {
        use shim::{TYPE_I64, TYPE_VOID};
        match self {
            Self::ChannelSend => TYPE_I64,
            Self::FsExists => TYPE_I64,
            Self::FsMkdir => TYPE_VOID,
            Self::FsReaddir => TYPE_I64,
            Self::FsStat => TYPE_I64,
            Self::FsUnlink => TYPE_VOID,
            Self::GetConfig => TYPE_I64,
            Self::HttpRequest => TYPE_I64,
            Self::KvGet => TYPE_I64,
            Self::KvSet => TYPE_VOID,
            Self::Log => TYPE_VOID,
            Self::ReadFile => TYPE_I64,
            Self::RegisterConnector => TYPE_I64,
            Self::WriteFile => TYPE_VOID,
        }
    }
}

pub fn register_host_functions(
    mut builder: PluginBuilder,
    user_data: UserData<HostState>,
) -> PluginBuilder {
    for func in WasmHostFunction::ALL {
        let ud = user_data.clone();

        let args = vec![extism::PTR; func.arg_count() as usize];
        let rets = if func.return_type() == shim::TYPE_I64 {
            vec![extism::PTR]
        } else {
            vec![]
        };

        builder = match func {
            WasmHostFunction::ChannelSend => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                channel::astrid_channel_send_impl,
            ),
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
            WasmHostFunction::GetConfig => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_get_config_impl)
            },
            WasmHostFunction::HttpRequest => {
                builder.with_function(func.name(), args, rets, ud, http::astrid_http_request_impl)
            },
            WasmHostFunction::KvGet => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_get_impl)
            },
            WasmHostFunction::KvSet => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_set_impl)
            },
            WasmHostFunction::Log => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_log_impl)
            },
            WasmHostFunction::ReadFile => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_read_file_impl)
            },
            WasmHostFunction::RegisterConnector => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                channel::astrid_register_connector_impl,
            ),
            WasmHostFunction::WriteFile => {
                builder.with_function(func.name(), args, rets, ud, fs::astrid_write_file_impl)
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
