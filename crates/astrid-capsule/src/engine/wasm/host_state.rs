//! Shared state for Extism host functions.
//!
//! [`HostState`] is wrapped in [`extism::UserData`] and shared across all
//! host function invocations for a single plugin instance.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Semaphore, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::capsule::CapsuleId;
use astrid_core::uplink::{InboundMessage, MAX_UPLINKS_PER_CAPSULE, UplinkDescriptor};
use astrid_storage::ScopedKvStore;
use astrid_storage::secret::SecretStore;

/// The lifecycle phase a capsule is currently executing in.
///
/// Set on [`HostState`] during `#[install]` or `#[upgrade]` dispatch.
/// The `astrid_elicit` host function checks this field and rejects calls
/// outside of a lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecyclePhase {
    /// First-time installation.
    Install,
    /// Upgrading from a previous version.
    Upgrade,
}

/// A pre-registered interceptor subscription for run-loop capsules.
///
/// Created during `WasmEngine::load()` when a capsule declares both
/// `run()` and `[[interceptor]]`. Maps a subscription handle ID (stored
/// in `HostState.subscriptions`) to the interceptor action name and topic.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InterceptorHandle {
    /// The subscription handle ID (key in `HostState.subscriptions`).
    pub handle_id: u64,
    /// The interceptor action name from the manifest.
    pub action: String,
    /// The event topic this interceptor subscribes to.
    pub topic: String,
}

use crate::engine::wasm::host::process::ProcessTracker;
use crate::security::CapsuleSecurityGate;

/// Shared state accessible to all host functions via `UserData<HostState>`.
pub struct HostState {
    /// The plugin this state belongs to.
    pub capsule_id: CapsuleId,
    /// Context of the current caller (set per-invocation by the dispatcher).
    pub caller_context: Option<astrid_events::ipc::IpcMessage>,
    /// The unique session UUID for this plugin's execution state.
    pub capsule_uuid: uuid::Uuid,
    /// Workspace root directory (file operations are confined here).
    pub workspace_root: PathBuf,
    /// The Virtual File System (VFS) instance for this plugin.
    pub vfs: Arc<dyn astrid_vfs::Vfs>,
    /// The root capability handle for the VFS.
    pub vfs_root_handle: astrid_capabilities::DirHandle,
    /// Global shared resources directory (`~/.astrid/shared/`). Paths prefixed
    /// with `global://` are resolved relative to this root.
    pub global_root: Option<PathBuf>,
    /// VFS instance for the global shared root. This is a direct `HostVfs` —
    /// writes are permanent (no OverlayVfs CoW layer).
    pub global_vfs: Option<Arc<dyn astrid_vfs::Vfs>>,
    /// Capability handle for the global shared VFS root.
    pub global_vfs_root_handle: Option<astrid_capabilities::DirHandle>,
    /// Concrete reference to the [`OverlayVfs`](astrid_vfs::OverlayVfs) for
    /// commit/rollback operations. `None` for non-overlay VFS configurations
    /// (e.g., tests with a plain `HostVfs`).
    pub overlay_vfs: Option<Arc<astrid_vfs::OverlayVfs>>,
    /// Reference to the ephemeral upper directory to keep it alive for the session.
    pub upper_dir: Option<Arc<tempfile::TempDir>>,
    /// Plugin-scoped KV store (`plugin:{capsule_id}` namespace).
    pub kv: ScopedKvStore,
    /// System Event Bus for IPC publish/subscribe.
    pub event_bus: astrid_events::EventBus,
    /// Rate limiter for IPC message publishing.
    pub ipc_limiter: astrid_events::ipc::IpcRateLimiter,
    /// Active event bus subscriptions for IPC events.
    pub subscriptions: HashMap<u64, astrid_events::EventReceiver>,
    /// Counter for issuing subscription handle IDs.
    pub next_subscription_id: u64,
    /// Plugin configuration from the manifest.
    pub config: HashMap<String, serde_json::Value>,
    /// IPC topic patterns this capsule is allowed to publish to.
    /// Empty means DENY ALL (fail-closed).
    pub ipc_publish_patterns: Vec<String>,
    /// IPC topic patterns this capsule is allowed to subscribe to.
    /// Empty means DENY ALL (fail-closed).
    pub ipc_subscribe_patterns: Vec<String>,
    /// Optional security gate for gated operations (HTTP, file I/O).
    pub security: Option<Arc<dyn CapsuleSecurityGate>>,
    /// Hook manager for executing user scripts synchronously via airlock.
    pub hook_manager: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// Shared capsule registry for `hooks::trigger` fan-out dispatch.
    ///
    /// When set, the `astrid_trigger_hook` host function can iterate the
    /// registry to find capsules with matching interceptors, invoke them,
    /// and collect responses. This is the kernel mechanism that WASM
    /// capsules use to dispatch hooks to other capsules.
    pub capsule_registry: Option<Arc<tokio::sync::RwLock<crate::registry::CapsuleRegistry>>>,
    /// Tokio runtime handle for bridging async operations in sync host functions.
    pub runtime_handle: tokio::runtime::Handle,
    /// Whether the plugin manifest declares `CapsuleCapability::Uplink`.
    ///
    /// Used to gate `astrid_register_uplink` — only uplink plugins
    /// are allowed to register uplinks.
    pub has_uplink_capability: bool,
    /// Sender for inbound messages from uplink plugins.
    ///
    /// Set during plugin loading when the manifest declares
    /// [`CapsuleCapability::Uplink`](crate::CapsuleCapability). Feeds into
    /// the gateway's inbound router.
    pub inbound_tx: Option<mpsc::Sender<InboundMessage>>,
    /// Uplinks registered by the WASM guest via `astrid_register_uplink`.
    pub registered_uplinks: Vec<UplinkDescriptor>,
    /// Optional natively bound unix listener.
    pub cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    /// Active, mapped UnixStreams from the socket listener.
    pub active_streams:
        std::collections::HashMap<u64, Arc<tokio::sync::Mutex<tokio::net::UnixStream>>>,
    /// Monotonic counter for stream handle IDs (avoids reuse after removal).
    /// Starts at 1 so that handle ID 0 is never issued — 0 is reserved as a
    /// sentinel / "no handle" value in the WASM ABI.
    pub next_stream_id: u64,
    /// Active lifecycle phase, if any. `None` during normal runtime.
    /// Set to `Some(Install)` or `Some(Upgrade)` during lifecycle dispatch.
    /// Gates the `astrid_elicit` host function.
    pub lifecycle_phase: Option<LifecyclePhase>,
    /// Secret store for capsule credentials (keychain with KV fallback).
    pub secret_store: Arc<dyn SecretStore>,
    /// Readiness signal sender for run-loop capsules.
    ///
    /// When the WASM guest calls `astrid_signal_ready`, the host sends `true`
    /// on this channel. The kernel waits on the corresponding receiver before
    /// loading dependent capsules.
    pub ready_tx: Option<watch::Sender<bool>>,
    /// Bounded concurrency semaphore for host function blocking calls.
    ///
    /// Limits the number of concurrent `block_in_place` / `block_on` operations
    /// across all capsules to prevent tokio thread-pool exhaustion.
    /// Created via [`default_host_semaphore`].
    pub host_semaphore: Arc<Semaphore>,
    /// Cooperative cancellation token for long-running host function calls.
    ///
    /// Triggered during capsule unload to unblock `ipc_recv`, `elicit`, and
    /// `net_accept`/`net_read`/`net_write` host functions that may be waiting on I/O.
    pub cancel_token: CancellationToken,
    /// Session token for authenticating CLI socket connections. Only set for
    /// the CLI proxy capsule (which has `net_bind` capability).
    pub session_token: Option<std::sync::Arc<astrid_core::session_token::SessionToken>>,
    /// Pre-registered interceptor subscription handles for run-loop capsules.
    ///
    /// Populated during `WasmEngine::load()` when a capsule declares both
    /// `run()` and `[[interceptor]]`. Each entry maps a subscription handle
    /// (in `self.subscriptions`) to the interceptor action name.
    pub interceptor_handles: Vec<InterceptorHandle>,
    /// Shared allowance store for capsule-level approval requests.
    ///
    /// When set, the `astrid_request_approval` host function can check
    /// existing allowances before prompting the user. Approvals with
    /// session/always scope create new allowances here.
    pub allowance_store: Option<std::sync::Arc<astrid_approval::AllowanceStore>>,
    /// Shared identity store for resolving platform users to `AstridUserId`.
    ///
    /// When `None`, identity host functions return an error.
    pub identity_store: Option<std::sync::Arc<dyn astrid_storage::IdentityStore>>,
    /// Tracks active child process PIDs for cancellation.
    ///
    /// Shared with the cancel listener background task. The spawn host function
    /// registers/unregisters PIDs; the listener calls `cancel_all()` when a
    /// `tool.v1.request.cancel` event arrives.
    pub process_tracker: Arc<ProcessTracker>,
}

impl HostState {
    /// Register a uplink descriptor (called from the host function).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The per-capsule uplink limit ([`MAX_UPLINKS_PER_CAPSULE`]) has been reached.
    /// - A uplink with the same name and platform already exists.
    pub fn register_uplink(&mut self, descriptor: UplinkDescriptor) -> Result<(), &'static str> {
        if self.registered_uplinks.len() >= MAX_UPLINKS_PER_CAPSULE {
            return Err("uplink registration limit reached");
        }
        // Reject duplicate name+platform combinations
        let duplicate = self
            .registered_uplinks
            .iter()
            .any(|c| c.name == descriptor.name && c.platform == descriptor.platform);
        if duplicate {
            return Err("duplicate uplink name and platform");
        }
        self.registered_uplinks.push(descriptor);
        Ok(())
    }

    /// Return the registered uplinks.
    #[must_use]
    pub fn uplinks(&self) -> &[UplinkDescriptor] {
        &self.registered_uplinks
    }

    /// Create the default host semaphore for bounding concurrent blocking calls.
    ///
    /// Reserves 2 threads for the tokio scheduler and event dispatch, with a
    /// minimum of 2 permits so capsules can always make progress.
    #[must_use]
    pub fn default_host_semaphore() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(
            std::thread::available_parallelism()
                .map(|n| n.get().saturating_sub(2).max(2))
                .unwrap_or(2),
        ))
    }

    /// Set the inbound message sender.
    pub fn set_inbound_tx(&mut self, tx: mpsc::Sender<InboundMessage>) {
        self.inbound_tx = Some(tx);
    }
}

impl std::fmt::Debug for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("capsule_id", &self.capsule_id)
            .field("workspace_root", &self.workspace_root)
            .field("vfs_root_handle", &self.vfs_root_handle)
            .field("has_global_root", &self.global_root.is_some())
            .field("has_security", &self.security.is_some())
            .field("has_uplink_capability", &self.has_uplink_capability)
            .field("has_inbound_tx", &self.inbound_tx.is_some())
            .field("registered_uplinks", &self.registered_uplinks.len())
            .field(
                "host_semaphore_permits",
                &self.host_semaphore.available_permits(),
            )
            .field("cancel_token_cancelled", &self.cancel_token.is_cancelled())
            .field("has_identity_store", &self.identity_store.is_some())
            .field("process_tracker", &self.process_tracker)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_state_debug_format() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "capsule:test").unwrap();
        let secret_store: Arc<dyn SecretStore> = Arc::new(astrid_storage::KvSecretStore::new(
            kv.clone(),
            rt.handle().clone(),
        ));

        let state = HostState {
            capsule_uuid: uuid::Uuid::new_v4(),
            caller_context: None,
            capsule_id: CapsuleId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            vfs: Arc::new(astrid_vfs::HostVfs::new()),
            vfs_root_handle: astrid_capabilities::DirHandle::new(),
            global_root: None,
            global_vfs: None,
            global_vfs_root_handle: None,
            overlay_vfs: None,
            upper_dir: None,
            kv,
            event_bus: astrid_events::EventBus::with_capacity(128),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            ipc_publish_patterns: Vec::new(),
            ipc_subscribe_patterns: Vec::new(),
            security: None,
            hook_manager: None,
            capsule_registry: None,
            runtime_handle: rt.handle().clone(),
            has_uplink_capability: false,
            inbound_tx: None,
            registered_uplinks: Vec::new(),
            cli_socket_listener: None,
            active_streams: std::collections::HashMap::new(),
            next_stream_id: 1,
            lifecycle_phase: None,
            secret_store: secret_store.clone(),
            ready_tx: None,
            host_semaphore: Arc::new(Semaphore::new(2)),
            cancel_token: CancellationToken::new(),
            session_token: None,
            interceptor_handles: Vec::new(),
            allowance_store: None,
            identity_store: None,
            process_tracker: Arc::new(ProcessTracker::new()),
        };

        let debug = format!("{state:?}");
        assert!(debug.contains("test"));
        assert!(debug.contains("has_security"));
        assert!(debug.contains("has_inbound_tx"));
        assert!(debug.contains("registered_uplinks"));
    }

    #[test]
    fn register_uplink_accumulates() {
        use crate::capsule::CapsuleId;
        use astrid_core::uplink::{UplinkCapabilities, UplinkProfile, UplinkSource};

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "capsule:test").unwrap();
        let secret_store: Arc<dyn SecretStore> = Arc::new(astrid_storage::KvSecretStore::new(
            kv.clone(),
            rt.handle().clone(),
        ));

        let mut state = HostState {
            capsule_uuid: uuid::Uuid::new_v4(),
            caller_context: None,
            capsule_id: CapsuleId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            vfs: Arc::new(astrid_vfs::HostVfs::new()),
            vfs_root_handle: astrid_capabilities::DirHandle::new(),
            global_root: None,
            global_vfs: None,
            global_vfs_root_handle: None,
            overlay_vfs: None,
            upper_dir: None,
            kv,
            event_bus: astrid_events::EventBus::with_capacity(128),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            ipc_publish_patterns: Vec::new(),
            ipc_subscribe_patterns: Vec::new(),
            security: None,
            hook_manager: None,
            capsule_registry: None,
            runtime_handle: rt.handle().clone(),
            has_uplink_capability: true,
            inbound_tx: None,
            registered_uplinks: Vec::new(),
            cli_socket_listener: None,
            active_streams: std::collections::HashMap::new(),
            next_stream_id: 1,
            lifecycle_phase: None,
            secret_store: secret_store.clone(),
            ready_tx: None,
            host_semaphore: Arc::new(Semaphore::new(2)),
            cancel_token: CancellationToken::new(),
            session_token: None,
            interceptor_handles: Vec::new(),
            allowance_store: None,
            identity_store: None,
            process_tracker: Arc::new(ProcessTracker::new()),
        };

        assert!(state.uplinks().is_empty());

        let desc = UplinkDescriptor::builder("test-conn", "discord")
            .source(UplinkSource::Wasm {
                capsule_id: "test".into(),
            })
            .capabilities(UplinkCapabilities::receive_only())
            .profile(UplinkProfile::Chat)
            .build();
        state.register_uplink(desc).unwrap();

        assert_eq!(state.uplinks().len(), 1);
        assert_eq!(state.uplinks()[0].name, "test-conn");
    }

    #[test]
    fn set_inbound_tx_stores_sender() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "capsule:test").unwrap();
        let secret_store: Arc<dyn SecretStore> = Arc::new(astrid_storage::KvSecretStore::new(
            kv.clone(),
            rt.handle().clone(),
        ));

        let mut state = HostState {
            capsule_uuid: uuid::Uuid::new_v4(),
            caller_context: None,
            capsule_id: CapsuleId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            vfs: Arc::new(astrid_vfs::HostVfs::new()),
            vfs_root_handle: astrid_capabilities::DirHandle::new(),
            global_root: None,
            global_vfs: None,
            global_vfs_root_handle: None,
            overlay_vfs: None,
            upper_dir: None,
            kv,
            event_bus: astrid_events::EventBus::with_capacity(128),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            ipc_publish_patterns: Vec::new(),
            ipc_subscribe_patterns: Vec::new(),
            security: None,
            hook_manager: None,
            capsule_registry: None,
            runtime_handle: rt.handle().clone(),
            has_uplink_capability: false,
            inbound_tx: None,
            registered_uplinks: Vec::new(),
            cli_socket_listener: None,
            active_streams: std::collections::HashMap::new(),
            next_stream_id: 1,
            lifecycle_phase: None,
            secret_store: secret_store.clone(),
            ready_tx: None,
            host_semaphore: Arc::new(Semaphore::new(2)),
            cancel_token: CancellationToken::new(),
            session_token: None,
            interceptor_handles: Vec::new(),
            allowance_store: None,
            identity_store: None,
            process_tracker: Arc::new(ProcessTracker::new()),
        };

        assert!(state.inbound_tx.is_none());

        let (tx, _rx) = mpsc::channel(256);
        state.set_inbound_tx(tx);

        assert!(state.inbound_tx.is_some());
    }

    #[test]
    fn register_uplink_rejects_at_limit() {
        use crate::capsule::CapsuleId;
        use astrid_core::uplink::{UplinkCapabilities, UplinkProfile, UplinkSource};

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "capsule:test").unwrap();
        let secret_store: Arc<dyn SecretStore> = Arc::new(astrid_storage::KvSecretStore::new(
            kv.clone(),
            rt.handle().clone(),
        ));

        let mut state = HostState {
            capsule_uuid: uuid::Uuid::new_v4(),
            caller_context: None,
            capsule_id: CapsuleId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            vfs: Arc::new(astrid_vfs::HostVfs::new()),
            vfs_root_handle: astrid_capabilities::DirHandle::new(),
            global_root: None,
            global_vfs: None,
            global_vfs_root_handle: None,
            overlay_vfs: None,
            upper_dir: None,
            kv,
            event_bus: astrid_events::EventBus::with_capacity(128),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            ipc_publish_patterns: Vec::new(),
            ipc_subscribe_patterns: Vec::new(),
            security: None,
            hook_manager: None,
            capsule_registry: None,
            runtime_handle: rt.handle().clone(),
            has_uplink_capability: true,
            inbound_tx: None,
            registered_uplinks: Vec::new(),
            cli_socket_listener: None,
            active_streams: std::collections::HashMap::new(),
            next_stream_id: 1,
            lifecycle_phase: None,
            secret_store: secret_store.clone(),
            ready_tx: None,
            host_semaphore: Arc::new(Semaphore::new(2)),
            cancel_token: CancellationToken::new(),
            session_token: None,
            interceptor_handles: Vec::new(),
            allowance_store: None,
            identity_store: None,
            process_tracker: Arc::new(ProcessTracker::new()),
        };

        for i in 0..MAX_UPLINKS_PER_CAPSULE {
            let desc = UplinkDescriptor::builder(format!("conn-{i}"), "discord")
                .source(UplinkSource::Wasm {
                    capsule_id: "test".into(),
                })
                .capabilities(UplinkCapabilities::receive_only())
                .profile(UplinkProfile::Chat)
                .build();
            assert!(state.register_uplink(desc).is_ok());
        }

        assert_eq!(state.uplinks().len(), MAX_UPLINKS_PER_CAPSULE);

        let extra = UplinkDescriptor::builder("over-limit", "discord")
            .source(UplinkSource::Wasm {
                capsule_id: "test".into(),
            })
            .capabilities(UplinkCapabilities::receive_only())
            .profile(UplinkProfile::Chat)
            .build();
        assert!(state.register_uplink(extra).is_err());
        assert_eq!(state.uplinks().len(), MAX_UPLINKS_PER_CAPSULE);
    }

    #[test]
    fn register_uplink_rejects_duplicate_name_and_platform() {
        use crate::capsule::CapsuleId;
        use astrid_core::uplink::{UplinkCapabilities, UplinkProfile, UplinkSource};

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "capsule:test").unwrap();
        let secret_store: Arc<dyn SecretStore> = Arc::new(astrid_storage::KvSecretStore::new(
            kv.clone(),
            rt.handle().clone(),
        ));

        let mut state = HostState {
            capsule_uuid: uuid::Uuid::new_v4(),
            caller_context: None,
            capsule_id: CapsuleId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            vfs: Arc::new(astrid_vfs::HostVfs::new()),
            vfs_root_handle: astrid_capabilities::DirHandle::new(),
            global_root: None,
            global_vfs: None,
            global_vfs_root_handle: None,
            overlay_vfs: None,
            upper_dir: None,
            kv,
            event_bus: astrid_events::EventBus::with_capacity(128),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            ipc_publish_patterns: Vec::new(),
            ipc_subscribe_patterns: Vec::new(),
            security: None,
            hook_manager: None,
            capsule_registry: None,
            runtime_handle: rt.handle().clone(),
            has_uplink_capability: true,
            inbound_tx: None,
            registered_uplinks: Vec::new(),
            cli_socket_listener: None,
            active_streams: std::collections::HashMap::new(),
            next_stream_id: 1,
            lifecycle_phase: None,
            secret_store: secret_store.clone(),
            ready_tx: None,
            host_semaphore: Arc::new(Semaphore::new(2)),
            cancel_token: CancellationToken::new(),
            session_token: None,
            interceptor_handles: Vec::new(),
            allowance_store: None,
            identity_store: None,
            process_tracker: Arc::new(ProcessTracker::new()),
        };

        let desc1 = UplinkDescriptor::builder("my-conn", "discord")
            .source(UplinkSource::Wasm {
                capsule_id: "test".into(),
            })
            .capabilities(UplinkCapabilities::receive_only())
            .profile(UplinkProfile::Chat)
            .build();
        assert!(state.register_uplink(desc1).is_ok());

        let desc2 = UplinkDescriptor::builder("my-conn", "discord")
            .source(UplinkSource::Wasm {
                capsule_id: "test".into(),
            })
            .capabilities(UplinkCapabilities::receive_only())
            .profile(UplinkProfile::Chat)
            .build();
        let err = state.register_uplink(desc2).unwrap_err();
        assert!(err.contains("duplicate"), "expected duplicate error: {err}");

        let desc3 = UplinkDescriptor::builder("my-conn", "telegram")
            .source(UplinkSource::Wasm {
                capsule_id: "test".into(),
            })
            .capabilities(UplinkCapabilities::receive_only())
            .profile(UplinkProfile::Chat)
            .build();
        assert!(state.register_uplink(desc3).is_ok());
        assert_eq!(state.uplinks().len(), 2);
    }
}
