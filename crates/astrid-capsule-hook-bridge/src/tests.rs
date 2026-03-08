//! Integration tests for the Hook Bridge capsule.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use astrid_capsule::capsule::{Capsule, CapsuleId, CapsuleState};
use astrid_capsule::context::CapsuleContext;
use astrid_capsule::error::{CapsuleError, CapsuleResult};
use astrid_capsule::manifest::{CapabilitiesDef, CapsuleManifest, InterceptorDef, PackageDef};
use astrid_capsule::registry::CapsuleRegistry;
use astrid_capsule::tool::CapsuleTool;
use astrid_events::{AstridEvent, EventBus, EventMetadata};

use crate::mapping::MergeSemantics;
use crate::merge_responses;

// ── Test helpers ────────────────────────────────────────────────────

/// Build a minimal `CapsuleManifest` for test mock capsules.
fn mock_manifest(name: &str, interceptors: Vec<InterceptorDef>) -> CapsuleManifest {
    CapsuleManifest {
        package: PackageDef {
            name: name.to_string(),
            version: "0.0.1".to_string(),
            description: None,
            authors: Vec::new(),
            repository: None,
            homepage: None,
            documentation: None,
            license: None,
            license_file: None,
            readme: None,
            keywords: Vec::new(),
            categories: Vec::new(),
            astrid_version: None,
            publish: None,
            include: None,
            exclude: None,
            metadata: None,
        },
        components: Vec::new(),
        dependencies: std::collections::HashMap::new(),
        capabilities: CapabilitiesDef::default(),
        env: std::collections::HashMap::new(),
        context_files: Vec::new(),
        commands: Vec::new(),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        uplinks: Vec::new(),
        llm_providers: Vec::new(),
        interceptors,
        cron_jobs: Vec::new(),
        tools: Vec::new(),
    }
}

// ── Mock capsules ───────────────────────────────────────────────────

/// A mock capsule that records interceptor invocations and returns
/// a configurable response.
struct MockPluginCapsule {
    id: CapsuleId,
    manifest: CapsuleManifest,
    invoked: Arc<AtomicBool>,
    response: Vec<u8>,
}

impl MockPluginCapsule {
    #[allow(clippy::needless_pass_by_value)]
    fn new(
        name: &str,
        hook_name: &str,
        invoked: Arc<AtomicBool>,
        response: serde_json::Value,
    ) -> Self {
        Self {
            id: CapsuleId::from_static(name),
            manifest: mock_manifest(
                name,
                vec![InterceptorDef {
                    event: hook_name.to_string(),
                    action: "handle_hook".to_string(),
                }],
            ),
            invoked,
            response: serde_json::to_vec(&response).unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Capsule for MockPluginCapsule {
    fn id(&self) -> &CapsuleId {
        &self.id
    }
    fn manifest(&self) -> &CapsuleManifest {
        &self.manifest
    }
    fn state(&self) -> CapsuleState {
        CapsuleState::Ready
    }
    async fn load(&mut self, _ctx: &CapsuleContext) -> CapsuleResult<()> {
        Ok(())
    }
    async fn unload(&mut self) -> CapsuleResult<()> {
        Ok(())
    }
    fn tools(&self) -> &[Arc<dyn CapsuleTool>] {
        &[]
    }
    fn invoke_interceptor(&self, _action: &str, _payload: &[u8]) -> CapsuleResult<Vec<u8>> {
        self.invoked.store(true, Ordering::SeqCst);
        Ok(self.response.clone())
    }
}

/// A mock capsule that always returns an error from `invoke_interceptor`.
struct FailingCapsule {
    id: CapsuleId,
    manifest: CapsuleManifest,
    invoked: Arc<AtomicBool>,
}

impl FailingCapsule {
    fn new(name: &str, hook_name: &str, invoked: Arc<AtomicBool>) -> Self {
        Self {
            id: CapsuleId::from_static(name),
            manifest: mock_manifest(
                name,
                vec![InterceptorDef {
                    event: hook_name.to_string(),
                    action: "handle_hook".to_string(),
                }],
            ),
            invoked,
        }
    }
}

#[async_trait]
impl Capsule for FailingCapsule {
    fn id(&self) -> &CapsuleId {
        &self.id
    }
    fn manifest(&self) -> &CapsuleManifest {
        &self.manifest
    }
    fn state(&self) -> CapsuleState {
        CapsuleState::Ready
    }
    async fn load(&mut self, _ctx: &CapsuleContext) -> CapsuleResult<()> {
        Ok(())
    }
    async fn unload(&mut self) -> CapsuleResult<()> {
        Ok(())
    }
    fn tools(&self) -> &[Arc<dyn CapsuleTool>] {
        &[]
    }
    fn invoke_interceptor(&self, _action: &str, _payload: &[u8]) -> CapsuleResult<Vec<u8>> {
        self.invoked.store(true, Ordering::SeqCst);
        Err(CapsuleError::ExecutionFailed("simulated WASM crash".into()))
    }
}

// ── Merge semantics unit tests ──────────────────────────────────────

#[test]
fn merge_none_returns_empty() {
    let responses = vec![serde_json::json!({"foo": "bar"})];
    let result = merge_responses(&responses, &MergeSemantics::None);
    assert_eq!(result, serde_json::json!({}));
}

#[test]
fn merge_tool_call_before_any_skip_wins() {
    let responses = vec![
        serde_json::json!({"skip": false}),
        serde_json::json!({"skip": true}),
        serde_json::json!({"skip": false}),
    ];
    let result = merge_responses(&responses, &MergeSemantics::ToolCallBefore);
    assert_eq!(result["skip"], true);
}

#[test]
fn merge_tool_call_before_last_modified_params_wins() {
    let responses = vec![
        serde_json::json!({"modified_params": {"a": 1}}),
        serde_json::json!({"modified_params": {"b": 2}}),
    ];
    let result = merge_responses(&responses, &MergeSemantics::ToolCallBefore);
    assert_eq!(result["modified_params"], serde_json::json!({"b": 2}));
}

#[test]
fn merge_tool_call_before_null_params_skipped() {
    let responses = vec![
        serde_json::json!({"modified_params": {"a": 1}}),
        serde_json::json!({"modified_params": null}),
    ];
    let result = merge_responses(&responses, &MergeSemantics::ToolCallBefore);
    // Null is skipped, so {"a": 1} wins.
    assert_eq!(result["modified_params"], serde_json::json!({"a": 1}));
}

#[test]
fn merge_tool_call_before_no_skip_defaults_false() {
    let responses = vec![serde_json::json!({})];
    let result = merge_responses(&responses, &MergeSemantics::ToolCallBefore);
    assert_eq!(result["skip"], false);
}

#[test]
fn merge_tool_call_before_skip_and_modified_params_from_different_plugins() {
    // Plugin A sets skip:true but no modified_params.
    // Plugin B provides modified_params but skip:false.
    // Merged result: skip=true (any-skip-wins), modified_params from B (last non-null).
    let responses = vec![
        serde_json::json!({"skip": true}),
        serde_json::json!({"skip": false, "modified_params": {"overridden": true}}),
    ];
    let result = merge_responses(&responses, &MergeSemantics::ToolCallBefore);
    assert_eq!(result["skip"], true, "any skip:true should win");
    assert_eq!(
        result["modified_params"],
        serde_json::json!({"overridden": true}),
        "modified_params from non-skipping plugin should still be present"
    );
}

#[test]
fn merge_last_non_null_takes_last() {
    let responses = vec![
        serde_json::json!({"modified_result": "first"}),
        serde_json::json!({"modified_result": "second"}),
    ];
    let result = merge_responses(
        &responses,
        &MergeSemantics::LastNonNull {
            field: "modified_result",
        },
    );
    assert_eq!(result["modified_result"], "second");
}

#[test]
fn merge_last_non_null_skips_null() {
    let responses = vec![
        serde_json::json!({"modified_result": "first"}),
        serde_json::json!({"modified_result": null}),
    ];
    let result = merge_responses(
        &responses,
        &MergeSemantics::LastNonNull {
            field: "modified_result",
        },
    );
    assert_eq!(result["modified_result"], "first");
}

#[test]
fn merge_last_non_null_empty_responses() {
    let responses: Vec<serde_json::Value> = Vec::new();
    let result = merge_responses(
        &responses,
        &MergeSemantics::LastNonNull {
            field: "modified_result",
        },
    );
    assert!(result.as_object().unwrap().is_empty());
}

// ── Integration tests ───────────────────────────────────────────────

#[tokio::test]
async fn hook_bridge_dispatches_session_start() {
    let invoked = Arc::new(AtomicBool::new(false));
    let plugin = MockPluginCapsule::new(
        "test-plugin",
        "session_start",
        Arc::clone(&invoked),
        serde_json::json!({}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(plugin)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));
    let registry_clone = Arc::clone(&registry);

    let handle = tokio::spawn(crate::dispatch_loop(Arc::clone(&bus), registry_clone));

    // Yield to let the dispatch loop subscribe.
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::SessionCreated {
        metadata: EventMetadata::new("test"),
        session_id: uuid::Uuid::new_v4(),
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        invoked.load(Ordering::SeqCst),
        "session_start hook should have been dispatched to the plugin"
    );

    handle.abort();
}

#[tokio::test]
async fn hook_bridge_skips_ipc_events() {
    let invoked = Arc::new(AtomicBool::new(false));
    let plugin = MockPluginCapsule::new(
        "test-plugin-ipc",
        "session_start",
        Arc::clone(&invoked),
        serde_json::json!({}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(plugin)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    // Publish an IPC event — should be skipped by the hook bridge.
    let msg = astrid_events::IpcMessage::new(
        "session_start",
        astrid_events::IpcPayload::Custom {
            data: serde_json::json!({}),
        },
        uuid::Uuid::nil(),
    );
    bus.publish(AstridEvent::Ipc {
        metadata: EventMetadata::new("test"),
        message: msg,
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        !invoked.load(Ordering::SeqCst),
        "IPC events should NOT trigger hook bridge dispatch"
    );

    handle.abort();
}

#[tokio::test]
async fn hook_bridge_skips_unmapped_events() {
    let invoked = Arc::new(AtomicBool::new(false));
    let plugin = MockPluginCapsule::new(
        "test-plugin-unmapped",
        "runtime_started",
        Arc::clone(&invoked),
        serde_json::json!({}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(plugin)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    // RuntimeStarted is not in the hook mapping table.
    bus.publish(AstridEvent::RuntimeStarted {
        metadata: EventMetadata::new("test"),
        version: "0.1.0".into(),
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        !invoked.load(Ordering::SeqCst),
        "Unmapped events should NOT trigger hook dispatch"
    );

    handle.abort();
}

#[tokio::test]
async fn hook_bridge_publishes_decision_for_before_tool_call() {
    let invoked = Arc::new(AtomicBool::new(false));
    let plugin = MockPluginCapsule::new(
        "test-plugin-tool",
        "before_tool_call",
        Arc::clone(&invoked),
        serde_json::json!({"skip": true}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(plugin)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    // Subscribe to the decision topic BEFORE starting the dispatch loop.
    let mut decision_receiver = bus.subscribe_topic("hook_bridge.before_tool_call.decision");

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::ToolCallStarted {
        metadata: EventMetadata::new("test"),
        call_id: uuid::Uuid::new_v4(),
        tool_name: "search".into(),
        server_name: None,
    });

    // Wait for the decision event.
    let decision = tokio::time::timeout(Duration::from_secs(2), decision_receiver.recv())
        .await
        .expect("timed out waiting for decision event")
        .expect("decision event should be published");

    if let AstridEvent::Ipc { message, .. } = &*decision {
        assert_eq!(message.topic, "hook_bridge.before_tool_call.decision");
        if let astrid_events::IpcPayload::RawJson(val) = &message.payload {
            assert_eq!(val["skip"], true);
        } else {
            panic!("Expected RawJson payload");
        }
    } else {
        panic!("Expected IPC event");
    }

    handle.abort();
}

#[tokio::test]
async fn hook_bridge_multiple_plugins_deterministic_merge() {
    // Register two plugins with IDs that sort deterministically:
    // "aaa-plugin" < "zzz-plugin", so zzz-plugin's response wins
    // in "last non-null" merge.
    let invoked_a = Arc::new(AtomicBool::new(false));
    let invoked_z = Arc::new(AtomicBool::new(false));

    let plugin_a = MockPluginCapsule::new(
        "aaa-plugin",
        "after_tool_call",
        Arc::clone(&invoked_a),
        serde_json::json!({"modified_result": "from_aaa"}),
    );
    let plugin_z = MockPluginCapsule::new(
        "zzz-plugin",
        "after_tool_call",
        Arc::clone(&invoked_z),
        serde_json::json!({"modified_result": "from_zzz"}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(plugin_a)).unwrap();
    registry.register(Box::new(plugin_z)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let mut decision_receiver = bus.subscribe_topic("hook_bridge.after_tool_call.decision");

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::ToolCallCompleted {
        metadata: EventMetadata::new("test"),
        call_id: uuid::Uuid::new_v4(),
        tool_name: "search".into(),
        duration_ms: 42,
    });

    let decision = tokio::time::timeout(Duration::from_secs(2), decision_receiver.recv())
        .await
        .expect("timed out waiting for decision event")
        .expect("decision event should be published");

    assert!(invoked_a.load(Ordering::SeqCst));
    assert!(invoked_z.load(Ordering::SeqCst));

    if let AstridEvent::Ipc { message, .. } = &*decision {
        if let astrid_events::IpcPayload::RawJson(val) = &message.payload {
            // Deterministic: sorted by CapsuleId, zzz-plugin is last,
            // so "from_zzz" wins.
            assert_eq!(
                val["modified_result"], "from_zzz",
                "Last capsule in sorted order should win (deterministic merge)"
            );
        } else {
            panic!("Expected RawJson payload");
        }
    } else {
        panic!("Expected IPC event");
    }

    handle.abort();
}

#[tokio::test]
async fn hook_bridge_fire_and_forget_no_decision_published() {
    let invoked = Arc::new(AtomicBool::new(false));
    let plugin = MockPluginCapsule::new(
        "test-plugin-fandf",
        "message_received",
        Arc::clone(&invoked),
        serde_json::json!({"some": "data"}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(plugin)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let mut decision_receiver = bus.subscribe_topic("hook_bridge.message_received.decision");

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::MessageReceived {
        metadata: EventMetadata::new("test"),
        message_id: uuid::Uuid::new_v4(),
        frontend: "cli".into(),
    });

    // Wait briefly — no decision event should arrive for fire-and-forget hooks.
    let result = tokio::time::timeout(Duration::from_millis(300), decision_receiver.recv()).await;

    assert!(
        result.is_err(),
        "Fire-and-forget hooks should NOT publish decision events"
    );

    // But the interceptor should still have been invoked.
    assert!(invoked.load(Ordering::SeqCst));

    handle.abort();
}

// ── Self-exclusion test ─────────────────────────────────────────────

#[tokio::test]
async fn hook_bridge_does_not_dispatch_to_itself() {
    // Register a capsule with ID "hook-bridge" (same as CAPSULE_ID)
    // that subscribes to "session_start". It should be skipped.
    let self_invoked = Arc::new(AtomicBool::new(false));
    let self_capsule = MockPluginCapsule::new(
        "hook-bridge",
        "session_start",
        Arc::clone(&self_invoked),
        serde_json::json!({}),
    );

    // Also register a legitimate plugin to verify dispatch still works.
    let other_invoked = Arc::new(AtomicBool::new(false));
    let other_capsule = MockPluginCapsule::new(
        "real-plugin",
        "session_start",
        Arc::clone(&other_invoked),
        serde_json::json!({}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(self_capsule)).unwrap();
    registry.register(Box::new(other_capsule)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::SessionCreated {
        metadata: EventMetadata::new("test"),
        session_id: uuid::Uuid::new_v4(),
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        !self_invoked.load(Ordering::SeqCst),
        "Hook bridge should NOT dispatch to itself"
    );
    assert!(
        other_invoked.load(Ordering::SeqCst),
        "Other plugins should still be dispatched to"
    );

    handle.abort();
}

// ── Error resilience tests ──────────────────────────────────────────

#[tokio::test]
async fn failing_capsule_does_not_block_others() {
    // Register a failing capsule and a normal capsule for the same hook.
    // The failing one should be warned about but the normal one should
    // still be invoked and its response should be used.
    let fail_invoked = Arc::new(AtomicBool::new(false));
    let ok_invoked = Arc::new(AtomicBool::new(false));

    // "aaa-fail" sorts before "zzz-ok", so the failing capsule runs first.
    let fail_capsule =
        FailingCapsule::new("aaa-fail", "before_tool_call", Arc::clone(&fail_invoked));
    let ok_capsule = MockPluginCapsule::new(
        "zzz-ok",
        "before_tool_call",
        Arc::clone(&ok_invoked),
        serde_json::json!({"skip": true}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(fail_capsule)).unwrap();
    registry.register(Box::new(ok_capsule)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let mut decision_receiver = bus.subscribe_topic("hook_bridge.before_tool_call.decision");

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::ToolCallStarted {
        metadata: EventMetadata::new("test"),
        call_id: uuid::Uuid::new_v4(),
        tool_name: "search".into(),
        server_name: None,
    });

    let decision = tokio::time::timeout(Duration::from_secs(2), decision_receiver.recv())
        .await
        .expect("timed out waiting for decision event")
        .expect("decision event should be published despite failing capsule");

    // Both capsules should have been called.
    assert!(fail_invoked.load(Ordering::SeqCst));
    assert!(ok_invoked.load(Ordering::SeqCst));

    // The ok capsule's response should be merged.
    if let AstridEvent::Ipc { message, .. } = &*decision {
        if let astrid_events::IpcPayload::RawJson(val) = &message.payload {
            assert_eq!(
                val["skip"], true,
                "ok capsule's skip:true should be in merged result"
            );
        } else {
            panic!("Expected RawJson payload");
        }
    } else {
        panic!("Expected IPC event");
    }

    handle.abort();
}

#[tokio::test]
async fn failing_capsule_does_not_block_fire_and_forget() {
    // A failing capsule in a fire-and-forget hook should not cause
    // any issues — the error is logged and execution continues.
    let fail_invoked = Arc::new(AtomicBool::new(false));
    let ok_invoked = Arc::new(AtomicBool::new(false));

    let fail_capsule =
        FailingCapsule::new("aaa-fail-fandf", "session_start", Arc::clone(&fail_invoked));
    let ok_capsule = MockPluginCapsule::new(
        "zzz-ok-fandf",
        "session_start",
        Arc::clone(&ok_invoked),
        serde_json::json!({}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(fail_capsule)).unwrap();
    registry.register(Box::new(ok_capsule)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::SessionCreated {
        metadata: EventMetadata::new("test"),
        session_id: uuid::Uuid::new_v4(),
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        fail_invoked.load(Ordering::SeqCst),
        "failing capsule should have been called"
    );
    assert!(
        ok_invoked.load(Ordering::SeqCst),
        "ok capsule should still be called after failure"
    );

    handle.abort();
}

// ── End-to-end before_tool_call with mixed responses ────────────────

#[tokio::test]
async fn e2e_before_tool_call_skip_and_params_from_different_plugins() {
    // Plugin A (aaa-skipper) sets skip:true.
    // Plugin B (zzz-modifier) provides modified_params but skip:false.
    // Merged: skip=true, modified_params from B.
    let invoked_a = Arc::new(AtomicBool::new(false));
    let invoked_b = Arc::new(AtomicBool::new(false));

    let skipper = MockPluginCapsule::new(
        "aaa-skipper",
        "before_tool_call",
        Arc::clone(&invoked_a),
        serde_json::json!({"skip": true}),
    );
    let modifier = MockPluginCapsule::new(
        "zzz-modifier",
        "before_tool_call",
        Arc::clone(&invoked_b),
        serde_json::json!({"skip": false, "modified_params": {"key": "value"}}),
    );

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(skipper)).unwrap();
    registry.register(Box::new(modifier)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    let mut decision_receiver = bus.subscribe_topic("hook_bridge.before_tool_call.decision");

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    bus.publish(AstridEvent::ToolCallStarted {
        metadata: EventMetadata::new("test"),
        call_id: uuid::Uuid::new_v4(),
        tool_name: "dangerous_tool".into(),
        server_name: None,
    });

    let decision = tokio::time::timeout(Duration::from_secs(2), decision_receiver.recv())
        .await
        .expect("timed out")
        .expect("decision should be published");

    if let AstridEvent::Ipc { message, .. } = &*decision {
        if let astrid_events::IpcPayload::RawJson(val) = &message.payload {
            assert_eq!(val["skip"], true, "any skip:true should propagate");
            assert_eq!(
                val["modified_params"],
                serde_json::json!({"key": "value"}),
                "modified_params from non-skipping plugin should be preserved"
            );
        } else {
            panic!("Expected RawJson payload");
        }
    } else {
        panic!("Expected IPC event");
    }

    handle.abort();
}

// ── Load/unload lifecycle tests ─────────────────────────────────────

#[tokio::test]
async fn hook_bridge_load_starts_dispatch_and_unload_stops_it() {
    use astrid_storage::{MemoryKvStore, ScopedKvStore};
    use std::path::PathBuf;

    let registry = Arc::new(RwLock::new(CapsuleRegistry::new()));
    let bus = Arc::new(EventBus::with_capacity(64));

    let kv_store = Arc::new(MemoryKvStore::new());
    let scoped_kv = ScopedKvStore::new(kv_store, "hook-bridge").unwrap();

    let ctx = CapsuleContext::new(
        PathBuf::from("/tmp/test"),
        None,
        scoped_kv,
        Arc::clone(&bus),
        None,
    );

    let mut capsule = crate::HookBridgeCapsule::new(Arc::clone(&registry));

    // Before load: state is Unloaded.
    assert!(
        matches!(capsule.state(), CapsuleState::Unloaded),
        "capsule should start in Unloaded state"
    );

    // Load: should transition to Ready and start the dispatch task.
    capsule.load(&ctx).await.expect("load should succeed");
    assert!(
        matches!(capsule.state(), CapsuleState::Ready),
        "capsule should be Ready after load"
    );

    // Verify the dispatch task is running by publishing an event and
    // checking that it gets processed.
    let invoked = Arc::new(AtomicBool::new(false));
    let plugin = MockPluginCapsule::new(
        "lifecycle-test-plugin",
        "session_start",
        Arc::clone(&invoked),
        serde_json::json!({}),
    );
    {
        let mut reg = registry.write().await;
        reg.register(Box::new(plugin)).unwrap();
    }

    tokio::task::yield_now().await;

    bus.publish(AstridEvent::SessionCreated {
        metadata: EventMetadata::new("test"),
        session_id: uuid::Uuid::new_v4(),
    });

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        invoked.load(Ordering::SeqCst),
        "dispatch task should be running after load"
    );

    // Unload: should transition to Unloaded and abort the dispatch task.
    capsule.unload().await.expect("unload should succeed");
    assert!(
        matches!(capsule.state(), CapsuleState::Unloaded),
        "capsule should be Unloaded after unload"
    );
}

// ── Zero-subscriber early return test ───────────────────────────────

#[tokio::test]
async fn hook_bridge_no_subscribers_no_decision_published() {
    // No plugins registered — the Hook Bridge should early-return
    // without publishing any decision event.
    let registry = Arc::new(RwLock::new(CapsuleRegistry::new()));
    let bus = Arc::new(EventBus::with_capacity(64));

    let mut decision_receiver = bus.subscribe_topic("hook_bridge.before_tool_call.decision");

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    // Publish a mapped event with merge semantics (not fire-and-forget).
    bus.publish(AstridEvent::ToolCallStarted {
        metadata: EventMetadata::new("test"),
        call_id: uuid::Uuid::new_v4(),
        tool_name: "search".into(),
        server_name: None,
    });

    // No decision should be published when there are zero subscribers.
    let result = tokio::time::timeout(Duration::from_millis(300), decision_receiver.recv()).await;
    assert!(
        result.is_err(),
        "no decision event should be published when there are zero subscribers"
    );

    handle.abort();
}

// ── Concurrent event dispatch test ──────────────────────────────────

#[tokio::test]
async fn hook_bridge_concurrent_events_dispatch_independently() {
    use std::sync::atomic::AtomicU32;

    /// A capsule that counts invocations instead of just recording a bool.
    struct CountingCapsule {
        id: CapsuleId,
        manifest: CapsuleManifest,
        count: Arc<AtomicU32>,
        response: Vec<u8>,
    }

    #[async_trait]
    impl Capsule for CountingCapsule {
        fn id(&self) -> &CapsuleId {
            &self.id
        }
        fn manifest(&self) -> &CapsuleManifest {
            &self.manifest
        }
        fn state(&self) -> CapsuleState {
            CapsuleState::Ready
        }
        async fn load(&mut self, _ctx: &CapsuleContext) -> CapsuleResult<()> {
            Ok(())
        }
        async fn unload(&mut self) -> CapsuleResult<()> {
            Ok(())
        }
        fn tools(&self) -> &[Arc<dyn CapsuleTool>] {
            &[]
        }
        fn invoke_interceptor(&self, _action: &str, _payload: &[u8]) -> CapsuleResult<Vec<u8>> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    let count = Arc::new(AtomicU32::new(0));
    let capsule = CountingCapsule {
        id: CapsuleId::from_static("counting-plugin"),
        manifest: mock_manifest(
            "counting-plugin",
            vec![InterceptorDef {
                event: "before_tool_call".to_string(),
                action: "handle_hook".to_string(),
            }],
        ),
        count: Arc::clone(&count),
        response: serde_json::to_vec(&serde_json::json!({"skip": false})).unwrap(),
    };

    let mut registry = CapsuleRegistry::new();
    registry.register(Box::new(capsule)).unwrap();
    let registry = Arc::new(RwLock::new(registry));

    let bus = Arc::new(EventBus::with_capacity(64));

    // Subscribe to collect all decision events.
    let mut decision_receiver = bus.subscribe_topic("hook_bridge.before_tool_call.decision");

    let handle = tokio::spawn(crate::dispatch_loop(
        Arc::clone(&bus),
        Arc::clone(&registry),
    ));
    tokio::task::yield_now().await;

    // Fire 3 ToolCallStarted events concurrently.
    for _ in 0..3 {
        bus.publish(AstridEvent::ToolCallStarted {
            metadata: EventMetadata::new("test"),
            call_id: uuid::Uuid::new_v4(),
            tool_name: "search".into(),
            server_name: None,
        });
    }

    // Collect all 3 decision events.
    let mut decisions = Vec::new();
    for _ in 0..3 {
        let decision = tokio::time::timeout(Duration::from_secs(2), decision_receiver.recv())
            .await
            .expect("timed out waiting for decision event")
            .expect("decision event should be published");
        decisions.push(decision);
    }

    assert_eq!(
        count.load(Ordering::SeqCst),
        3,
        "interceptor should have been invoked exactly 3 times (once per event)"
    );
    assert_eq!(
        decisions.len(),
        3,
        "3 independent decision events should have been published"
    );

    handle.abort();
}
