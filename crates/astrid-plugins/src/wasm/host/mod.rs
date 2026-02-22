/// Channel communications with host capabilities.
pub mod channel;
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

use crate::wasm::host_state::HostState;
use extism::{PluginBuilder, UserData, ValType};

/// Registry of explicitly supported capability functions exposed to the WASM Runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmHostFunction {
    /// Send a capability message over a shared pipeline.
    ChannelSend,
    /// Predict existence of a filepath.
    FsExists,
    /// Create a new directory.
    FsMkdir,
    /// Read contents of a directory.
    FsReaddir,
    /// Request metadata status of a file.
    FsStat,
    /// Unlink (Delete) a file.
    FsUnlink,
    /// Retrieve system execution config.
    GetConfig,
    /// Execute an HTTP request payload.
    HttpRequest,
    /// Publish an IPC message to the host event bus.
    IpcPublish,
    /// Subscribe to IPC events via the host event bus.
    IpcSubscribe,
    /// Unsubscribe from an IPC event subscription.
    IpcUnsubscribe,
    /// Retrieve a persistence KV.
    KvGet,
    /// Write a persistence KV.
    KvSet,
    /// Output standard log statements.
    Log,
    /// Open and read a filesystem file.
    ReadFile,
    /// Register a custom system connector to the host.
    RegisterConnector,
    /// Mutate a filesystem file.
    WriteFile,
}

impl WasmHostFunction {
    /// Ordered precisely as expected by the `QuickJS` shim kernel.
    pub const ALL: [Self; 17] = [
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
        Self::IpcPublish,
        Self::IpcSubscribe,
        Self::IpcUnsubscribe,
    ];

    /// Convert a raw integer index mapping back to a strongly typed enum variant.
    #[must_use]
    pub fn from_index(idx: usize) -> Option<Self> {
        Self::ALL.get(idx).copied()
    }

    /// Retrieve the universally bound identifier string used across the extension ABI.
    #[must_use]
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
            Self::IpcPublish => "astrid_ipc_publish",
            Self::IpcSubscribe => "astrid_ipc_subscribe",
            Self::IpcUnsubscribe => "astrid_ipc_unsubscribe",
            Self::KvGet => "astrid_kv_get",
            Self::KvSet => "astrid_kv_set",
            Self::Log => "astrid_log",
            Self::ReadFile => "astrid_read_file",
            Self::RegisterConnector => "astrid_register_connector",
            Self::WriteFile => "astrid_write_file",
        }
    }

    /// Compute the number of strongly typed arguments required for this WASM function.
    #[must_use]
    pub fn arg_count(self) -> usize {
        match self {
            Self::FsExists
            | Self::FsMkdir
            | Self::FsReaddir
            | Self::FsStat
            | Self::FsUnlink
            | Self::GetConfig
            | Self::HttpRequest
            | Self::IpcSubscribe
            | Self::IpcUnsubscribe
            | Self::KvGet
            | Self::ReadFile => 1,
            Self::KvSet | Self::Log | Self::WriteFile | Self::IpcPublish => 2,
            Self::ChannelSend | Self::RegisterConnector => 3,
        }
    }

    /// Defines the low-level expected ABI return integer block type.
    #[must_use]
    pub fn return_type(self) -> i32 {
        use shim::{TYPE_I64, TYPE_VOID};
        match self {
            Self::FsMkdir
            | Self::FsUnlink
            | Self::IpcPublish
            | Self::IpcUnsubscribe
            | Self::KvSet
            | Self::Log
            | Self::WriteFile => TYPE_VOID,
            Self::ChannelSend
            | Self::FsExists
            | Self::FsReaddir
            | Self::FsStat
            | Self::GetConfig
            | Self::HttpRequest
            | Self::IpcSubscribe
            | Self::KvGet
            | Self::ReadFile
            | Self::RegisterConnector => TYPE_I64,
        }
    }
}

/// Hydrates an isolated WASM Extism Runtime with capabilities bound securely to the `HostState` lifecycle environment.
#[allow(clippy::too_many_lines)]
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
