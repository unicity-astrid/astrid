//! Tests for `host_state.rs`. Split to keep `host_state.rs` under the
//! 1000-line CI threshold. Included via `#[path]` from its sibling.

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
        wasi_ctx: wasmtime_wasi::WasiCtxBuilder::new().build(),
        resource_table: wasmtime::component::ResourceTable::new(),
        store_limits: wasmtime::StoreLimitsBuilder::new().build(),
        principal: astrid_core::PrincipalId::default(),
        capsule_uuid: uuid::Uuid::new_v4(),
        caller_context: None,
        invocation_kv: None,
        capsule_log: None,
        capsule_id: CapsuleId::from_static("test"),
        workspace_root: PathBuf::from("/tmp"),
        vfs: Arc::new(astrid_vfs::HostVfs::new()),
        vfs_root_handle: astrid_capabilities::DirHandle::new(),
        home: None,
        tmp: None,
        invocation_home: None,
        invocation_tmp: None,
        invocation_secret_store: None,
        invocation_capsule_log: None,
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
        active_http_streams: HashMap::new(),
        next_http_stream_id: 1,
        lifecycle_phase: None,
        secret_store: secret_store.clone(),
        ready_tx: None,
        host_semaphore: Arc::new(Semaphore::new(2)),
        cancel_token: CancellationToken::new(),
        session_token: None,
        interceptor_handles: Vec::new(),
        allowance_store: None,
        identity_store: None,
        background_processes: HashMap::new(),
        next_process_id: 1,
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
        wasi_ctx: wasmtime_wasi::WasiCtxBuilder::new().build(),
        resource_table: wasmtime::component::ResourceTable::new(),
        store_limits: wasmtime::StoreLimitsBuilder::new().build(),
        principal: astrid_core::PrincipalId::default(),
        capsule_uuid: uuid::Uuid::new_v4(),
        caller_context: None,
        invocation_kv: None,
        capsule_log: None,
        capsule_id: CapsuleId::from_static("test"),
        workspace_root: PathBuf::from("/tmp"),
        vfs: Arc::new(astrid_vfs::HostVfs::new()),
        vfs_root_handle: astrid_capabilities::DirHandle::new(),
        home: None,
        tmp: None,
        invocation_home: None,
        invocation_tmp: None,
        invocation_secret_store: None,
        invocation_capsule_log: None,
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
        active_http_streams: HashMap::new(),
        next_http_stream_id: 1,
        lifecycle_phase: None,
        secret_store: secret_store.clone(),
        ready_tx: None,
        host_semaphore: Arc::new(Semaphore::new(2)),
        cancel_token: CancellationToken::new(),
        session_token: None,
        interceptor_handles: Vec::new(),
        allowance_store: None,
        identity_store: None,
        background_processes: HashMap::new(),
        next_process_id: 1,
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
        wasi_ctx: wasmtime_wasi::WasiCtxBuilder::new().build(),
        resource_table: wasmtime::component::ResourceTable::new(),
        store_limits: wasmtime::StoreLimitsBuilder::new().build(),
        principal: astrid_core::PrincipalId::default(),
        capsule_uuid: uuid::Uuid::new_v4(),
        caller_context: None,
        invocation_kv: None,
        capsule_log: None,
        capsule_id: CapsuleId::from_static("test"),
        workspace_root: PathBuf::from("/tmp"),
        vfs: Arc::new(astrid_vfs::HostVfs::new()),
        vfs_root_handle: astrid_capabilities::DirHandle::new(),
        home: None,
        tmp: None,
        invocation_home: None,
        invocation_tmp: None,
        invocation_secret_store: None,
        invocation_capsule_log: None,
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
        active_http_streams: HashMap::new(),
        next_http_stream_id: 1,
        lifecycle_phase: None,
        secret_store: secret_store.clone(),
        ready_tx: None,
        host_semaphore: Arc::new(Semaphore::new(2)),
        cancel_token: CancellationToken::new(),
        session_token: None,
        interceptor_handles: Vec::new(),
        allowance_store: None,
        identity_store: None,
        background_processes: HashMap::new(),
        next_process_id: 1,
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
        wasi_ctx: wasmtime_wasi::WasiCtxBuilder::new().build(),
        resource_table: wasmtime::component::ResourceTable::new(),
        store_limits: wasmtime::StoreLimitsBuilder::new().build(),
        principal: astrid_core::PrincipalId::default(),
        capsule_uuid: uuid::Uuid::new_v4(),
        caller_context: None,
        invocation_kv: None,
        capsule_log: None,
        capsule_id: CapsuleId::from_static("test"),
        workspace_root: PathBuf::from("/tmp"),
        vfs: Arc::new(astrid_vfs::HostVfs::new()),
        vfs_root_handle: astrid_capabilities::DirHandle::new(),
        home: None,
        tmp: None,
        invocation_home: None,
        invocation_tmp: None,
        invocation_secret_store: None,
        invocation_capsule_log: None,
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
        active_http_streams: HashMap::new(),
        next_http_stream_id: 1,
        lifecycle_phase: None,
        secret_store: secret_store.clone(),
        ready_tx: None,
        host_semaphore: Arc::new(Semaphore::new(2)),
        cancel_token: CancellationToken::new(),
        session_token: None,
        interceptor_handles: Vec::new(),
        allowance_store: None,
        identity_store: None,
        background_processes: HashMap::new(),
        next_process_id: 1,
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
        wasi_ctx: wasmtime_wasi::WasiCtxBuilder::new().build(),
        resource_table: wasmtime::component::ResourceTable::new(),
        store_limits: wasmtime::StoreLimitsBuilder::new().build(),
        principal: astrid_core::PrincipalId::default(),
        capsule_uuid: uuid::Uuid::new_v4(),
        caller_context: None,
        invocation_kv: None,
        capsule_log: None,
        capsule_id: CapsuleId::from_static("test"),
        workspace_root: PathBuf::from("/tmp"),
        vfs: Arc::new(astrid_vfs::HostVfs::new()),
        vfs_root_handle: astrid_capabilities::DirHandle::new(),
        home: None,
        tmp: None,
        invocation_home: None,
        invocation_tmp: None,
        invocation_secret_store: None,
        invocation_capsule_log: None,
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
        active_http_streams: HashMap::new(),
        next_http_stream_id: 1,
        lifecycle_phase: None,
        secret_store: secret_store.clone(),
        ready_tx: None,
        host_semaphore: Arc::new(Semaphore::new(2)),
        cancel_token: CancellationToken::new(),
        session_token: None,
        interceptor_handles: Vec::new(),
        allowance_store: None,
        identity_store: None,
        background_processes: HashMap::new(),
        next_process_id: 1,
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

// ---------------------------------------------------------------------
// effective_* accessor precedence (#661)
// ---------------------------------------------------------------------
//
// Chain tests in host/sys.rs and host/elicit.rs cover the end-to-end
// wiring for `log` and `has_secret`. These direct unit tests pin the
// accessor contract itself so it survives future chain-test refactors:
// when an invocation value is installed, the accessor returns it; when
// it's cleared, the accessor falls back to the load-time value.

/// Minimal HostState fixture for accessor-precedence tests. Duplicates
/// the per-test `HostState { ... }` literal because `HostState` has no
/// `Default` impl and the other tests in this module each inline their
/// own. A dedicated builder is tracked as follow-up cleanup (see the
/// `#662 follow-ups` umbrella issue).
fn make_effective_accessor_state() -> HostState {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv = ScopedKvStore::new(store, "capsule:test").unwrap();
    let secret_store: Arc<dyn SecretStore> = Arc::new(astrid_storage::KvSecretStore::new(
        kv.clone(),
        rt.handle().clone(),
    ));

    HostState {
        wasi_ctx: wasmtime_wasi::WasiCtxBuilder::new().build(),
        resource_table: wasmtime::component::ResourceTable::new(),
        store_limits: wasmtime::StoreLimitsBuilder::new().build(),
        principal: astrid_core::PrincipalId::default(),
        capsule_uuid: uuid::Uuid::new_v4(),
        caller_context: None,
        invocation_kv: None,
        capsule_log: None,
        capsule_id: CapsuleId::from_static("test"),
        workspace_root: PathBuf::from("/tmp"),
        vfs: Arc::new(astrid_vfs::HostVfs::new()),
        vfs_root_handle: astrid_capabilities::DirHandle::new(),
        home: None,
        tmp: None,
        invocation_home: None,
        invocation_tmp: None,
        invocation_secret_store: None,
        invocation_capsule_log: None,
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
        active_streams: HashMap::new(),
        next_stream_id: 1,
        active_http_streams: HashMap::new(),
        next_http_stream_id: 1,
        lifecycle_phase: None,
        secret_store,
        ready_tx: None,
        host_semaphore: Arc::new(Semaphore::new(2)),
        cancel_token: CancellationToken::new(),
        session_token: None,
        interceptor_handles: Vec::new(),
        allowance_store: None,
        identity_store: None,
        background_processes: HashMap::new(),
        next_process_id: 1,
        process_tracker: Arc::new(ProcessTracker::new()),
    }
}

#[test]
fn effective_secret_store_prefers_invocation_over_load_time() {
    let mut state = make_effective_accessor_state();

    // Load-time store pointer (snapshotted as raw `*const` for identity
    // comparison — `Arc::ptr_eq` would require an identical `Arc`).
    let owner_ptr = Arc::as_ptr(&state.secret_store);
    assert!(
        std::ptr::eq(Arc::as_ptr(state.effective_secret_store()), owner_ptr),
        "with no invocation store installed, effective_* returns the load-time store"
    );

    // Install a distinct invocation store; accessor must switch.
    let alice_kv = ScopedKvStore::new(
        Arc::new(astrid_storage::MemoryKvStore::new()),
        "capsule:alice",
    )
    .unwrap();
    let alice_store: Arc<dyn SecretStore> = Arc::new(astrid_storage::KvSecretStore::new(
        alice_kv,
        state.runtime_handle.clone(),
    ));
    let alice_ptr = Arc::as_ptr(&alice_store);
    state.invocation_secret_store = Some(alice_store);

    assert!(
        std::ptr::eq(Arc::as_ptr(state.effective_secret_store()), alice_ptr),
        "with invocation store installed, accessor returns the invocation store"
    );
    assert!(
        !std::ptr::eq(Arc::as_ptr(state.effective_secret_store()), owner_ptr),
        "owner's store must not be returned while invocation is installed"
    );

    // Clear; falls back.
    state.invocation_secret_store = None;
    assert!(
        std::ptr::eq(Arc::as_ptr(state.effective_secret_store()), owner_ptr),
        "after clear, accessor falls back to the load-time store"
    );
}

#[test]
fn effective_capsule_log_prefers_invocation_over_load_time() {
    let tmp = tempfile::tempdir().unwrap();
    let owner_path = tmp.path().join("owner.log");
    let alice_path = tmp.path().join("alice.log");
    let owner_log: Arc<std::sync::Mutex<std::fs::File>> = Arc::new(std::sync::Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&owner_path)
            .unwrap(),
    ));
    let alice_log: Arc<std::sync::Mutex<std::fs::File>> = Arc::new(std::sync::Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&alice_path)
            .unwrap(),
    ));

    let mut state = make_effective_accessor_state();

    // No logs installed → None.
    assert!(state.effective_capsule_log().is_none());

    // Only load-time installed → returns load-time.
    state.capsule_log = Some(Arc::clone(&owner_log));
    assert!(
        Arc::ptr_eq(state.effective_capsule_log().unwrap(), &owner_log),
        "only load-time log installed"
    );

    // Both installed → returns invocation.
    state.invocation_capsule_log = Some(Arc::clone(&alice_log));
    assert!(
        Arc::ptr_eq(state.effective_capsule_log().unwrap(), &alice_log),
        "invocation wins when both are installed"
    );

    // Clear invocation → falls back to load-time.
    state.invocation_capsule_log = None;
    assert!(
        Arc::ptr_eq(state.effective_capsule_log().unwrap(), &owner_log),
        "falls back to load-time after invocation clear"
    );
}
