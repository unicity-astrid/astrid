/// Capsule-level approval requests.
pub(crate) mod approval;
/// Cron scheduling.
pub(crate) mod cron;
/// Elicit lifecycle API (install/upgrade user input collection).
pub(crate) mod elicit;
/// File system operations for plugins.
pub(crate) mod fs;
/// HTTP network executions for plugins.
pub mod http;
/// Identity operations (resolve, link, create user).
pub(crate) mod identity;
/// Inter-Process Communication bus.
pub(crate) mod ipc;
/// Key-Value persistent storage primitives.
pub(crate) mod kv;
pub(crate) mod net;
/// Process spawning and sandboxing.
pub mod process;
/// `QuickJS` ABI definitions.
pub(crate) mod shim;
/// System configuration primitives.
pub mod sys;
/// Uplink communications with host capabilities.
pub(crate) mod uplink;
/// Utility functions for WASM host implementations.
pub(crate) mod util;

use crate::engine::wasm::host_state::HostState;
use extism::{PluginBuilder, UserData, ValType};

/// Registry of explicitly supported capability functions exposed to the WASM Runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmHostFunction {
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
    IpcRecv,
    UplinkRegister,
    UplinkSend,
    KvGet,
    KvSet,
    KvDelete,
    KvListKeys,
    KvClearPrefix,
    GetConfig,
    NetBindUnix,
    NetAccept,
    NetRead,
    NetWrite,
    GetCaller,
    HttpRequest,
    TriggerHook,
    Log,
    CronSchedule,
    CronCancel,
    SpawnHost,
    Elicit,
    HasSecret,
    SignalReady,
    ClockMs,
    GetInterceptorHandles,
    RequestApproval,
    IdentityResolve,
    IdentityLink,
    IdentityUnlink,
    IdentityCreateUser,
    IdentityListLinks,
    CheckCapsuleCapability,
    SpawnBackgroundHost,
    ReadProcessLogsHost,
    KillProcessHost,
    NetCloseStream,
    NetPollAccept,
    HttpStreamStart,
    HttpStreamRead,
    HttpStreamClose,
}

impl WasmHostFunction {
    pub(crate) const ALL: [Self; 51] = [
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
        Self::IpcRecv,
        Self::UplinkRegister,
        Self::UplinkSend,
        Self::KvGet,
        Self::KvSet,
        Self::KvDelete,
        Self::KvListKeys,
        Self::KvClearPrefix,
        Self::GetConfig,
        Self::GetCaller,
        Self::HttpRequest,
        Self::TriggerHook,
        Self::Log,
        Self::CronSchedule,
        Self::CronCancel,
        Self::SpawnHost,
        Self::NetBindUnix,
        Self::NetAccept,
        Self::NetRead,
        Self::NetWrite,
        Self::Elicit,
        Self::HasSecret,
        Self::SignalReady,
        Self::ClockMs,
        Self::GetInterceptorHandles,
        Self::RequestApproval,
        Self::IdentityResolve,
        Self::IdentityLink,
        Self::IdentityUnlink,
        Self::IdentityCreateUser,
        Self::IdentityListLinks,
        Self::CheckCapsuleCapability,
        Self::SpawnBackgroundHost,
        Self::ReadProcessLogsHost,
        Self::KillProcessHost,
        Self::NetCloseStream,
        Self::NetPollAccept,
        Self::HttpStreamStart,
        Self::HttpStreamRead,
        Self::HttpStreamClose,
    ];

    #[must_use]
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    #[must_use]
    pub(crate) fn name(self) -> &'static str {
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
            Self::IpcRecv => "astrid_ipc_recv",
            Self::UplinkRegister => "astrid_uplink_register",
            Self::UplinkSend => "astrid_uplink_send",
            Self::KvGet => "astrid_kv_get",
            Self::KvSet => "astrid_kv_set",
            Self::KvDelete => "astrid_kv_delete",
            Self::KvListKeys => "astrid_kv_list_keys",
            Self::KvClearPrefix => "astrid_kv_clear_prefix",
            Self::GetConfig => "astrid_get_config",
            Self::NetBindUnix => "astrid_net_bind_unix",
            Self::NetAccept => "astrid_net_accept",
            Self::NetRead => "astrid_net_read",
            Self::NetWrite => "astrid_net_write",
            Self::GetCaller => "astrid_get_caller",
            Self::HttpRequest => "astrid_http_request",
            Self::TriggerHook => "astrid_trigger_hook",
            Self::Log => "astrid_log",
            Self::CronSchedule => "astrid_cron_schedule",
            Self::CronCancel => "astrid_cron_cancel",
            Self::SpawnHost => "astrid_spawn_host",
            Self::Elicit => "astrid_elicit",
            Self::HasSecret => "astrid_has_secret",
            Self::SignalReady => "astrid_signal_ready",
            Self::ClockMs => "astrid_clock_ms",
            Self::GetInterceptorHandles => "astrid_get_interceptor_handles",
            Self::RequestApproval => "astrid_request_approval",
            Self::IdentityResolve => "astrid_identity_resolve",
            Self::IdentityLink => "astrid_identity_link",
            Self::IdentityUnlink => "astrid_identity_unlink",
            Self::IdentityCreateUser => "astrid_identity_create_user",
            Self::IdentityListLinks => "astrid_identity_list_links",
            Self::CheckCapsuleCapability => "astrid_check_capsule_capability",
            Self::SpawnBackgroundHost => "astrid_spawn_background_host",
            Self::ReadProcessLogsHost => "astrid_read_process_logs_host",
            Self::KillProcessHost => "astrid_kill_process_host",
            Self::NetCloseStream => "astrid_net_close_stream",
            Self::NetPollAccept => "astrid_net_poll_accept",
            Self::HttpStreamStart => "astrid_http_stream_start",
            Self::HttpStreamRead => "astrid_http_stream_read",
            Self::HttpStreamClose => "astrid_http_stream_close",
        }
    }

    #[must_use]
    pub(crate) fn arg_count(self) -> usize {
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
            | Self::KvGet
            | Self::KvDelete
            | Self::KvListKeys
            | Self::KvClearPrefix
            | Self::GetConfig
            | Self::HttpRequest
            | Self::SpawnHost
            | Self::CronCancel
            | Self::NetBindUnix
            | Self::NetAccept
            | Self::NetRead
            | Self::TriggerHook
            | Self::Elicit
            | Self::HasSecret
            | Self::RequestApproval
            | Self::IdentityResolve
            | Self::IdentityLink
            | Self::IdentityUnlink
            | Self::IdentityCreateUser
            | Self::IdentityListLinks
            | Self::CheckCapsuleCapability
            | Self::SpawnBackgroundHost
            | Self::ReadProcessLogsHost
            | Self::KillProcessHost
            | Self::NetCloseStream
            | Self::NetPollAccept
            | Self::HttpStreamStart
            | Self::HttpStreamRead
            | Self::HttpStreamClose => 1,
            Self::WriteFile
            | Self::IpcPublish
            | Self::IpcRecv
            | Self::KvSet
            | Self::Log
            | Self::NetWrite => 2,
            Self::UplinkRegister | Self::UplinkSend | Self::CronSchedule => 3,
            Self::GetCaller | Self::SignalReady | Self::ClockMs | Self::GetInterceptorHandles => 0,
        }
    }

    #[must_use]
    pub(crate) fn return_type(self) -> i32 {
        use shim::{TYPE_I64, TYPE_VOID};
        match self {
            Self::FsMkdir
            | Self::FsUnlink
            | Self::WriteFile
            | Self::NetWrite
            | Self::IpcPublish
            | Self::IpcUnsubscribe
            | Self::KvSet
            | Self::KvDelete
            | Self::Log
            | Self::CronSchedule
            | Self::CronCancel
            | Self::SignalReady
            | Self::NetCloseStream
            | Self::HttpStreamClose => TYPE_VOID,
            Self::FsExists
            | Self::FsReaddir
            | Self::FsStat
            | Self::ReadFile
            | Self::IpcSubscribe
            | Self::IpcPoll
            | Self::IpcRecv
            | Self::UplinkRegister
            | Self::UplinkSend
            | Self::KvGet
            | Self::KvListKeys
            | Self::KvClearPrefix
            | Self::GetConfig
            | Self::SpawnHost
            | Self::HttpRequest
            | Self::GetCaller
            | Self::TriggerHook
            | Self::NetBindUnix
            | Self::NetAccept
            | Self::NetRead
            | Self::Elicit
            | Self::HasSecret
            | Self::ClockMs
            | Self::GetInterceptorHandles
            | Self::RequestApproval
            | Self::IdentityResolve
            | Self::IdentityLink
            | Self::IdentityUnlink
            | Self::IdentityCreateUser
            | Self::IdentityListLinks
            | Self::CheckCapsuleCapability
            | Self::SpawnBackgroundHost
            | Self::ReadProcessLogsHost
            | Self::KillProcessHost
            | Self::NetPollAccept
            | Self::HttpStreamStart
            | Self::HttpStreamRead => TYPE_I64,
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
            WasmHostFunction::IpcRecv => {
                builder.with_function(func.name(), args, rets, ud, ipc::astrid_ipc_recv_impl)
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

            WasmHostFunction::KvGet => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_get_impl)
            },
            WasmHostFunction::KvSet => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_set_impl)
            },
            WasmHostFunction::KvDelete => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_delete_impl)
            },
            WasmHostFunction::KvListKeys => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_list_keys_impl)
            },
            WasmHostFunction::KvClearPrefix => {
                builder.with_function(func.name(), args, rets, ud, kv::astrid_kv_clear_prefix_impl)
            },
            WasmHostFunction::NetBindUnix => {
                builder.with_function(func.name(), args, rets, ud, net::astrid_net_bind_unix_impl)
            },
            WasmHostFunction::NetAccept => {
                builder.with_function(func.name(), args, rets, ud, net::astrid_net_accept_impl)
            },
            WasmHostFunction::NetRead => {
                builder.with_function(func.name(), args, rets, ud, net::astrid_net_read_impl)
            },
            WasmHostFunction::NetWrite => {
                builder.with_function(func.name(), args, rets, ud, net::astrid_net_write_impl)
            },
            WasmHostFunction::GetConfig => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_get_config_impl)
            },
            WasmHostFunction::GetCaller => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_get_caller_impl)
            },
            WasmHostFunction::HttpRequest => {
                builder.with_function(func.name(), args, rets, ud, http::astrid_http_request_impl)
            },
            WasmHostFunction::TriggerHook => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_trigger_hook_impl)
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
            WasmHostFunction::SpawnHost => {
                builder.with_function(func.name(), args, rets, ud, process::astrid_spawn_host_impl)
            },
            WasmHostFunction::Elicit => {
                builder.with_function(func.name(), args, rets, ud, elicit::astrid_elicit_impl)
            },
            WasmHostFunction::HasSecret => {
                builder.with_function(func.name(), args, rets, ud, elicit::astrid_has_secret_impl)
            },
            WasmHostFunction::SignalReady => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_signal_ready_impl)
            },
            WasmHostFunction::ClockMs => {
                builder.with_function(func.name(), args, rets, ud, sys::astrid_clock_ms_impl)
            },
            WasmHostFunction::GetInterceptorHandles => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                ipc::astrid_get_interceptor_handles_impl,
            ),
            WasmHostFunction::RequestApproval => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                approval::astrid_request_approval_impl,
            ),
            WasmHostFunction::IdentityResolve => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                identity::astrid_identity_resolve_impl,
            ),
            WasmHostFunction::IdentityLink => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                identity::astrid_identity_link_impl,
            ),
            WasmHostFunction::IdentityUnlink => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                identity::astrid_identity_unlink_impl,
            ),
            WasmHostFunction::IdentityCreateUser => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                identity::astrid_identity_create_user_impl,
            ),
            WasmHostFunction::IdentityListLinks => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                identity::astrid_identity_list_links_impl,
            ),
            WasmHostFunction::CheckCapsuleCapability => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                sys::astrid_check_capsule_capability_impl,
            ),
            WasmHostFunction::SpawnBackgroundHost => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                process::astrid_spawn_background_host_impl,
            ),
            WasmHostFunction::ReadProcessLogsHost => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                process::astrid_read_process_logs_host_impl,
            ),
            WasmHostFunction::KillProcessHost => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                process::astrid_kill_process_host_impl,
            ),
            WasmHostFunction::NetCloseStream => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                net::astrid_net_close_stream_impl,
            ),
            WasmHostFunction::NetPollAccept => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                net::astrid_net_poll_accept_impl,
            ),
            WasmHostFunction::HttpStreamStart => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                http::astrid_http_stream_start_impl,
            ),
            WasmHostFunction::HttpStreamRead => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                http::astrid_http_stream_read_impl,
            ),
            WasmHostFunction::HttpStreamClose => builder.with_function(
                func.name(),
                args,
                rets,
                ud,
                http::astrid_http_stream_close_impl,
            ),
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

#[cfg(test)]
mod tests {
    use super::WasmHostFunction;

    /// The shim dispatches by index into the ALL array. New variants MUST be
    /// appended at the end. This test locks the indices of all existing
    /// variants to catch accidental reordering.
    #[test]
    fn shim_index_stability() {
        assert_eq!(
            WasmHostFunction::from_index(46),
            Some(WasmHostFunction::NetCloseStream)
        );
        assert_eq!(
            WasmHostFunction::from_index(47),
            Some(WasmHostFunction::NetPollAccept)
        );
        assert_eq!(
            WasmHostFunction::from_index(48),
            Some(WasmHostFunction::HttpStreamStart)
        );
        assert_eq!(
            WasmHostFunction::from_index(49),
            Some(WasmHostFunction::HttpStreamRead)
        );
        assert_eq!(
            WasmHostFunction::from_index(50),
            Some(WasmHostFunction::HttpStreamClose)
        );
        // Sentinel: index beyond ALL returns None.
        assert_eq!(WasmHostFunction::from_index(51), None);
    }

    /// Every variant in the enum must be present in the ALL array exactly once.
    #[test]
    fn all_array_covers_every_variant() {
        let all = &WasmHostFunction::ALL;
        // If a variant is added to the enum but not ALL, this count will be
        // wrong and the assertion on length will fail.
        assert_eq!(all.len(), 51);

        // Check for duplicates.
        let mut seen = std::collections::HashSet::new();
        for func in all {
            assert!(
                seen.insert(func.name()),
                "duplicate entry in ALL: {}",
                func.name()
            );
        }
    }

    #[test]
    fn net_close_stream_properties() {
        let f = WasmHostFunction::NetCloseStream;
        assert_eq!(f.name(), "astrid_net_close_stream");
        assert_eq!(f.arg_count(), 1);
        assert_eq!(f.return_type(), super::shim::TYPE_VOID);
    }

    #[test]
    fn net_poll_accept_properties() {
        let f = WasmHostFunction::NetPollAccept;
        assert_eq!(f.name(), "astrid_net_poll_accept");
        assert_eq!(f.arg_count(), 1);
        assert_eq!(f.return_type(), super::shim::TYPE_I64);
    }

    /// Verify that legacy indices (0..42) still map to their expected functions.
    /// Catches accidental insertion in the middle of the ALL array.
    #[test]
    fn legacy_index_stability() {
        // Spot-check a few well-known positions.
        assert_eq!(
            WasmHostFunction::from_index(0),
            Some(WasmHostFunction::FsExists)
        );
        assert_eq!(
            WasmHostFunction::from_index(27),
            Some(WasmHostFunction::NetBindUnix)
        );
        assert_eq!(
            WasmHostFunction::from_index(28),
            Some(WasmHostFunction::NetAccept)
        );
        assert_eq!(
            WasmHostFunction::from_index(42),
            Some(WasmHostFunction::CheckCapsuleCapability)
        );
    }
}
