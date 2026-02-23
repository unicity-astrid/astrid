//! End-to-end WASM plugin integration test.
//!
//! Loads the `test-all-endpoints` WASM fixture (compiled from the
//! `test-plugin-guest` Rust crate) and exercises every host function endpoint.
//!
//! The WASM fixture is automatically compiled on first use via
//! `cargo build --target wasm32-unknown-unknown` on the guest crate.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Once};

use astrid_core::plugin_abi::{ToolDefinition, ToolInput, ToolOutput};
use astrid_plugins::PluginId;
use astrid_plugins::wasm::host::register_host_functions;
use astrid_plugins::wasm::host_state::HostState;
use astrid_storage::kv::ScopedKvStore;
use extism::{Manifest, PluginBuilder, UserData, Wasm};
use tokio::sync::mpsc;

/// Path to the compiled WASM fixture.
fn wasm_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-all-endpoints.wasm")
}

static BUILD_FIXTURE: Once = Once::new();

/// Build the `test-plugin-guest` crate to WASM and copy to the fixtures directory.
/// Uses `Once` to ensure compilation happens at most once per test process.
fn build_fixture() {
    BUILD_FIXTURE.call_once(|| {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let guest_dir = manifest_dir.parent().unwrap().join("test-plugin-guest");
        let fixture_dir = manifest_dir.join("tests/fixtures");

        let status = std::process::Command::new("cargo")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .current_dir(&guest_dir)
            .status()
            .expect("failed to invoke cargo for test-plugin-guest");
        assert!(
            status.success(),
            "failed to compile test-plugin-guest to WASM"
        );

        let src = guest_dir.join("target/wasm32-unknown-unknown/release/test_plugin_guest.wasm");
        std::fs::create_dir_all(&fixture_dir).expect("create fixtures dir");
        std::fs::copy(&src, fixture_dir.join("test-all-endpoints.wasm"))
            .expect("copy WASM fixture");
    });
}

/// Build an Extism plugin from the test fixture with all host functions registered.
fn build_test_plugin(
    workspace_root: &Path,
    config: HashMap<String, serde_json::Value>,
) -> extism::Plugin {
    build_test_plugin_with_security(workspace_root, config, None)
}

fn build_test_plugin_with_security(
    workspace_root: &Path,
    config: HashMap<String, serde_json::Value>,
    security: Option<Arc<dyn astrid_plugins::security::PluginSecurityGate>>,
) -> extism::Plugin {
    let wasm_bytes = std::fs::read(wasm_fixture_path()).expect("read WASM fixture");

    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv =
        ScopedKvStore::new(store, "plugin:test-all-endpoints").expect("create scoped KV store");

    let root_handle = astrid_capabilities::DirHandle::new();
    let host_state = HostState {
        plugin_uuid: uuid::Uuid::new_v4(),
        plugin_id: PluginId::from_static("test-all-endpoints"),
        workspace_root: workspace_root.to_path_buf(),
        vfs: std::sync::Arc::new({
            let v = astrid_vfs::HostVfs::new();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(v.register_dir(root_handle.clone(), workspace_root.to_path_buf()))
                    .expect("Failed to register VFS dir");
            });
            v
        }),
        vfs_root_handle: root_handle,
        upper_dir: None,
        kv,
        event_bus: astrid_events::EventBus::with_capacity(128),
        ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
        subscriptions: std::collections::HashMap::new(),
        next_subscription_id: 1,
        config,
        security,
        runtime_handle: tokio::runtime::Handle::current(),
        has_connector_capability: false,
        inbound_tx: None,
        registered_connectors: Vec::new(),
    };
    let user_data = UserData::new(host_state);

    let extism_wasm = Wasm::data(wasm_bytes);
    let extism_manifest = Manifest::new([extism_wasm]);

    let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
    let builder = register_host_functions(builder, user_data);
    builder.build().expect("build Extism plugin")
}

/// Build an Extism plugin with Connector capability enabled and an inbound channel.
///
/// Returns the plugin and the receiver end of the inbound message channel.
fn build_connector_plugin(
    workspace_root: &Path,
) -> (
    extism::Plugin,
    mpsc::Receiver<astrid_core::connector::InboundMessage>,
) {
    let wasm_bytes = std::fs::read(wasm_fixture_path()).expect("read WASM fixture");

    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv = ScopedKvStore::new(store, "plugin:test-connector").expect("create scoped KV store");

    let (tx, rx) = mpsc::channel(256);

    let root_handle = astrid_capabilities::DirHandle::new();
    let host_state = HostState {
        plugin_uuid: uuid::Uuid::new_v4(),
        plugin_id: PluginId::from_static("test-connector"),
        workspace_root: workspace_root.to_path_buf(),
        vfs: std::sync::Arc::new({
            let v = astrid_vfs::HostVfs::new();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(v.register_dir(root_handle.clone(), workspace_root.to_path_buf()))
                    .expect("Failed to register VFS dir");
            });
            v
        }),
        vfs_root_handle: root_handle,
        upper_dir: None,
        kv,
        event_bus: astrid_events::EventBus::with_capacity(128),
        ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
        subscriptions: std::collections::HashMap::new(),
        next_subscription_id: 1,
        config: HashMap::new(),
        security: None,
        runtime_handle: tokio::runtime::Handle::current(),
        has_connector_capability: true,
        inbound_tx: Some(tx),
        registered_connectors: Vec::new(),
    };
    let user_data = UserData::new(host_state);

    let extism_wasm = Wasm::data(wasm_bytes);
    let extism_manifest = Manifest::new([extism_wasm]);

    let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
    let builder = register_host_functions(builder, user_data);
    let plugin = builder.build().expect("build Extism plugin");

    (plugin, rx)
}

/// Call `describe-tools` and parse the result.
///
/// Uses `block_in_place` because host functions may call `handle.block_on()`
/// for async KV operations, which panics if called from a tokio worker thread.
fn describe_tools(plugin: &mut extism::Plugin) -> Vec<ToolDefinition> {
    let result = tokio::task::block_in_place(|| {
        plugin
            .call::<&str, String>("describe-tools", "")
            .expect("describe-tools should succeed")
    });
    serde_json::from_str(&result).expect("parse tool definitions")
}

/// Call `execute-tool` with the given tool name and arguments.
///
/// Uses `block_in_place` because host functions may call `handle.block_on()`
/// for async KV operations, which panics if called from a tokio worker thread.
fn execute_tool(plugin: &mut extism::Plugin, name: &str, args: &serde_json::Value) -> ToolOutput {
    let input = ToolInput {
        name: name.to_string(),
        arguments: serde_json::to_string(&args).unwrap(),
    };
    let input_json = serde_json::to_string(&input).unwrap();
    let result = tokio::task::block_in_place(|| {
        plugin
            .call::<&str, String>("execute-tool", &input_json)
            .unwrap_or_else(|e| panic!("execute-tool({name}) should succeed: {e}"))
    });
    serde_json::from_str(&result)
        .unwrap_or_else(|e| panic!("parse ToolOutput for {name}: {e}\nraw: {result}"))
}

/// Like `execute_tool` but returns `Err` if the Extism call itself fails
/// (e.g. host function rejects the call). This is needed for testing that
/// host-level security gates produce errors.
fn try_execute_tool(
    plugin: &mut extism::Plugin,
    name: &str,
    args: &serde_json::Value,
) -> Result<ToolOutput, String> {
    let input = ToolInput {
        name: name.to_string(),
        arguments: serde_json::to_string(&args).unwrap(),
    };
    let input_json = serde_json::to_string(&input).unwrap();
    let result = tokio::task::block_in_place(|| {
        plugin
            .call::<&str, String>("execute-tool", &input_json)
            .map_err(|e| format!("{e:?}"))
    })?;
    serde_json::from_str(&result).map_err(|e| format!("parse ToolOutput: {e}\nraw: {result}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discover_all_tools() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-discover");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let tools = describe_tools(&mut plugin);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    assert!(names.contains(&"test-log"), "missing test-log tool");
    assert!(names.contains(&"test-config"), "missing test-config tool");
    assert!(names.contains(&"test-kv"), "missing test-kv tool");
    assert!(
        names.contains(&"test-file-write"),
        "missing test-file-write tool"
    );
    assert!(
        names.contains(&"test-file-read"),
        "missing test-file-read tool"
    );
    assert!(
        names.contains(&"test-roundtrip"),
        "missing test-roundtrip tool"
    );
    assert!(
        names.contains(&"test-register-connector"),
        "missing test-register-connector tool"
    );
    assert!(
        names.contains(&"test-channel-send"),
        "missing test-channel-send tool"
    );

    assert_eq!(
        tools.len(),
        14,
        "expected exactly 14 tools, got {}",
        tools.len()
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_host_rejects_huge_log() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-malicious-log");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let result = try_execute_tool(&mut plugin, "test-malicious-log", &serde_json::json!({}));

    // It should return an error because the host function traps on limit violation
    assert!(result.is_err(), "host function must reject oversized log");
    let err = result.unwrap_err();
    assert!(err.contains("wasm backtrace"), "unexpected error: {err}");
    assert!(err.contains("astrid_log"), "should fail in astrid_log");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_host_rejects_huge_kv() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-malicious-kv");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let result = try_execute_tool(&mut plugin, "test-malicious-kv", &serde_json::json!({}));

    // It should return an error because the host function traps on limit violation
    assert!(
        result.is_err(),
        "host function must reject oversized KV payload"
    );
    let err = result.unwrap_err();
    assert!(err.contains("wasm backtrace"), "unexpected error: {err}");
    assert!(
        err.contains("astrid_kv_set"),
        "should fail in astrid_kv_set"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_log_all_levels() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-log");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let output = execute_tool(
        &mut plugin,
        "test-log",
        &serde_json::json!({ "message": "hello from e2e" }),
    );

    assert!(!output.is_error, "test-log should succeed");
    assert!(
        output.content.contains("hello from e2e"),
        "output should echo the message: {}",
        output.content
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_config_reads_baked_value() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-config");
    let _ = std::fs::create_dir_all(&workspace);

    let mut config = HashMap::new();
    config.insert("api_key".into(), serde_json::json!("sk-test-123"));
    let mut plugin = build_test_plugin(&workspace, config);

    // Read existing key
    let output = execute_tool(
        &mut plugin,
        "test-config",
        &serde_json::json!({ "key": "api_key" }),
    );
    assert!(!output.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["found"], true);
    assert_eq!(parsed["value"], "sk-test-123");

    // Read non-existent key
    let output = execute_tool(
        &mut plugin,
        "test-config",
        &serde_json::json!({ "key": "nonexistent" }),
    );
    assert!(!output.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["found"], false);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_kv_set_and_get() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-kv");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let output = execute_tool(
        &mut plugin,
        "test-kv",
        &serde_json::json!({ "key": "greeting", "value": "hello world" }),
    );

    assert!(!output.is_error, "test-kv should succeed");
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["key"], "greeting");
    assert_eq!(parsed["written"], "hello world");
    assert_eq!(parsed["read_back"], "hello world");
    assert_eq!(parsed["match"], true);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_file_write_and_read() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-file");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    // Write a file
    let output = execute_tool(
        &mut plugin,
        "test-file-write",
        &serde_json::json!({ "path": "test-output.txt", "content": "written by WASM plugin" }),
    );
    assert!(!output.is_error, "test-file-write should succeed");

    // Verify file exists on disk
    let written_path = workspace.join("test-output.txt");
    assert!(written_path.exists(), "file should exist on disk");
    let disk_content = std::fs::read_to_string(&written_path).unwrap();
    assert_eq!(disk_content, "written by WASM plugin");

    // Read it back via the plugin
    let output = execute_tool(
        &mut plugin,
        "test-file-read",
        &serde_json::json!({ "path": "test-output.txt" }),
    );
    assert!(!output.is_error, "test-file-read should succeed");
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["content"], "written by WASM plugin");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_host_file_read_write_absolute_paths() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-file-abs");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    // Write a file using a WASI-style absolute path
    let output = execute_tool(
        &mut plugin,
        "test-file-write",
        &serde_json::json!({ "path": "/test-output-abs.txt", "content": "written by WASM plugin using absolute path" }),
    );
    assert!(
        !output.is_error,
        "test-file-write with absolute path should succeed: {}",
        output.content
    );

    // Verify file exists on disk (it should be placed relative to the workspace root)
    let written_path = workspace.join("test-output-abs.txt");
    assert!(
        written_path.exists(),
        "file should exist on disk at correct path"
    );
    let disk_content = std::fs::read_to_string(&written_path).unwrap();
    assert_eq!(disk_content, "written by WASM plugin using absolute path");

    // Read it back via the plugin using the absolute path
    let output = execute_tool(
        &mut plugin,
        "test-file-read",
        &serde_json::json!({ "path": "/test-output-abs.txt" }),
    );
    assert!(
        !output.is_error,
        "test-file-read with absolute path should succeed: {}",
        output.content
    );
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(
        parsed["content"],
        "written by WASM plugin using absolute path"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_kv_roundtrip_structured_data() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-roundtrip");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let test_data = serde_json::json!({
        "name": "astrid",
        "version": 42,
        "features": ["wasm", "mcp", "hooks"],
        "nested": { "deep": true }
    });

    let output = execute_tool(
        &mut plugin,
        "test-roundtrip",
        &serde_json::json!({ "data": test_data }),
    );

    assert!(!output.is_error, "test-roundtrip should succeed");
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["integrity"], true, "data integrity check failed");
    assert_eq!(parsed["original"]["name"], "astrid");
    assert_eq!(parsed["round_tripped"]["version"], 42);
    assert_eq!(parsed["round_tripped"]["features"][0], "wasm");
    assert_eq!(parsed["round_tripped"]["nested"]["deep"], true);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_tool_returns_error() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-unknown");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let output = execute_tool(&mut plugin, "nonexistent-tool", &serde_json::json!({}));

    assert!(output.is_error, "unknown tool should return error");
    assert!(
        output.content.contains("unknown tool"),
        "error should mention unknown tool: {}",
        output.content
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

// ---------------------------------------------------------------------------
// Connector E2E tests (require has_connector_capability = true)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_register_connector_returns_uuid() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-register-conn");
    let _ = std::fs::create_dir_all(&workspace);
    let (mut plugin, _rx) = build_connector_plugin(&workspace);

    let output = execute_tool(
        &mut plugin,
        "test-register-connector",
        &serde_json::json!({ "name": "my-discord-bot", "platform": "discord", "profile": "chat" }),
    );

    assert!(
        !output.is_error,
        "register-connector should succeed: {}",
        output.content
    );
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["registered"], true);
    assert_eq!(parsed["name"], "my-discord-bot");
    assert_eq!(parsed["platform"], "discord");

    // Verify the connector_id is a valid UUID
    let connector_id = parsed["connector_id"].as_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(connector_id).is_ok(),
        "connector_id should be a valid UUID: {connector_id}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_channel_send_delivers_message() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-channel-send");
    let _ = std::fs::create_dir_all(&workspace);
    let (mut plugin, mut rx) = build_connector_plugin(&workspace);

    let output = execute_tool(
        &mut plugin,
        "test-channel-send",
        &serde_json::json!({
            "connector_name": "test-bot",
            "platform": "telegram",
            "user_id": "user-42",
            "message": "hello from WASM"
        }),
    );

    assert!(
        !output.is_error,
        "channel-send should succeed: {}",
        output.content
    );
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["send_result"]["ok"], true);
    assert_eq!(parsed["user_id"], "user-42");
    assert_eq!(parsed["message"], "hello from WASM");

    // Verify the message was received on the inbound channel
    let msg = rx
        .try_recv()
        .expect("should have received an inbound message");
    assert_eq!(msg.platform_user_id, "user-42");
    assert_eq!(msg.content, "hello from WASM");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_register_connector_rejected_without_capability() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-no-cap");
    let _ = std::fs::create_dir_all(&workspace);
    // Use the standard plugin builder (has_connector_capability = false)
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let result = try_execute_tool(
        &mut plugin,
        "test-register-connector",
        &serde_json::json!({ "name": "bad-conn", "platform": "discord", "profile": "chat" }),
    );

    // The host function rejects at the Extism level (before the guest can return ToolOutput).
    // Extism wraps host function errors in a WASM backtrace; verify the call failed.
    assert!(
        result.is_err(),
        "register-connector without capability should fail"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_ipc_publish_and_subscribe() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-ipc");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, HashMap::new());

    let output = execute_tool(
        &mut plugin,
        "test-ipc",
        &serde_json::json!({
            "topic": "test.topic.123",
            "payload": "{\"msg\":\"hello ipc\"}"
        }),
    );

    assert!(
        !output.is_error,
        "ipc test should succeed: {}",
        output.content
    );
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["topic"], "test.topic.123");
    assert_eq!(parsed["payload"], "{\"msg\":\"hello ipc\"}");
    assert!(parsed["subscription_handle"].as_str().is_some());
    assert_eq!(parsed["unsubscribed"], true);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_ipc_limits() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-ipc-limits");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, std::collections::HashMap::new());

    // Test 1: Publish large payload
    let output1 = try_execute_tool(
        &mut plugin,
        "test-ipc-limits",
        &serde_json::json!({
            "test_type": "publish_large"
        }),
    );

    assert!(output1.is_err(), "large publish should fail");
    let err_str = output1.unwrap_err().clone();
    assert!(
        err_str.contains("Payload too large"),
        "unexpected error message: {err_str}"
    );

    // Test 2: Subscribe loop
    let output2 = try_execute_tool(
        &mut plugin,
        "test-ipc-limits",
        &serde_json::json!({
            "test_type": "subscribe_loop"
        }),
    );

    assert!(output2.is_err(), "subscribe loop past 128 should fail");
    let err_str2 = output2.unwrap_err().clone();
    assert!(
        err_str2.contains("Subscription limit reached"),
        "unexpected error message: {err_str2}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[cfg(feature = "http")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_host_rejects_invalid_http_headers() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-malicious-http");
    let _ = std::fs::create_dir_all(&workspace);
    let mut plugin = build_test_plugin(&workspace, std::collections::HashMap::new());

    let result = try_execute_tool(
        &mut plugin,
        "test-malicious-http-headers",
        &serde_json::json!({}),
    );

    // It should return an error because the host function rejects invalid headers gracefully
    assert!(
        result.is_err(),
        "host function must reject invalid HTTP headers"
    );
    let err = result.unwrap_err();
    assert!(err.contains("invalid header"), "unexpected error: {err}");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[cfg(feature = "http")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http_restricted_headers_filtered() {
    build_fixture();
    let workspace = std::env::temp_dir().join("e2e-wasm-http-headers");
    let _ = std::fs::create_dir_all(&workspace);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_handle = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0; 4096];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);

            // reqwest uses lowercase header keys
            assert!(
                !req.to_lowercase().contains("host: malicious.com"),
                "host header injected!"
            );
            assert!(
                !req.to_lowercase().contains("connection: upgrade"),
                "connection header injected!"
            );
            assert!(
                !req.to_lowercase().contains("upgrade: websocket"),
                "upgrade header injected!"
            );
            assert!(
                !req.to_lowercase().contains("content-length: 999"),
                "content-length header injected!"
            );
            assert!(
                !req.to_lowercase().contains("transfer-encoding: chunked"),
                "transfer-encoding header injected!"
            );
            assert!(
                req.to_lowercase().contains("x-custom-header: allowed"),
                "custom header missing!"
            );

            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    let mut plugin = build_test_plugin(&workspace, std::collections::HashMap::new());

    let request_json = serde_json::json!({
        "method": "GET",
        "url": format!("http://127.0.0.1:{}/test", port),
        "headers": [
            {"key": "Host", "value": "malicious.com"},
            {"key": "Connection", "value": "upgrade"},
            {"key": "Upgrade", "value": "websocket"},
            {"key": "Content-Length", "value": "999"},
            {"key": "Transfer-Encoding", "value": "chunked"},
            {"key": "X-Custom-Header", "value": "allowed"}
        ]
    });

    let result = try_execute_tool(
        &mut plugin,
        "test-http",
        &serde_json::json!({ "request": serde_json::to_string(&request_json).unwrap() }),
    );

    assert!(result.is_ok(), "http request should succeed");
    let out = result.unwrap();
    assert!(!out.is_error, "http tool error: {}", out.content);
    assert!(
        out.content.contains("\"status\":200"),
        "response should be 200 OK"
    );
    assert!(
        out.content.contains("\"body\":\"OK\""),
        "response body should be OK"
    );

    server_handle.await.expect("Server task panicked");
    let _ = std::fs::remove_dir_all(&workspace);
}

#[derive(Default)]
struct CapturingSecurityGate {
    captured_paths: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl astrid_plugins::security::PluginSecurityGate for CapturingSecurityGate {
    async fn check_http_request(
        &self,
        _plugin_id: &str,
        _method: &str,
        _url: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn check_file_read(&self, _plugin_id: &str, path: &str) -> Result<(), String> {
        self.captured_paths.lock().unwrap().push(path.to_string());
        Ok(())
    }

    async fn check_file_write(&self, _plugin_id: &str, path: &str) -> Result<(), String> {
        self.captured_paths.lock().unwrap().push(path.to_string());
        Ok(())
    }

    async fn check_connector_register(
        &self,
        _plugin_id: &str,
        _connector_name: &str,
        _platform: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_host_file_read_write_symlink_bypass() {
    build_fixture();

    let workspace = std::env::temp_dir().join("e2e-wasm-file-symlink");
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).unwrap();

    // Create a scenario where a symlink points to a location outside the apparent "public" directory,
    // but still inside the workspace. The SecurityGate should evaluate the final canonicalized physical path.
    let public_dir = workspace.join("public");
    let private_dir = workspace.join("private");
    std::fs::create_dir_all(&public_dir).unwrap();
    std::fs::create_dir_all(&private_dir).unwrap();

    let secret_file = private_dir.join("secret.txt");
    std::fs::write(&secret_file, "top secret").unwrap();

    // Symlink: public/link -> ../private
    #[cfg(unix)]
    std::os::unix::fs::symlink("../private", public_dir.join("link")).unwrap();

    #[cfg(windows)]
    std::os::windows::fs::symlink_dir("../private", public_dir.join("link")).unwrap();

    let gate = std::sync::Arc::new(CapturingSecurityGate::default());
    let mut plugin = build_test_plugin_with_security(
        &workspace,
        std::collections::HashMap::new(),
        Some(gate.clone() as std::sync::Arc<dyn astrid_plugins::security::PluginSecurityGate>),
    );

    // WASM plugin requests to read public/link/secret.txt
    let output = execute_tool(
        &mut plugin,
        "test-file-read",
        &serde_json::json!({ "path": "public/link/secret.txt" }),
    );
    assert!(
        !output.is_error,
        "test-file-read through symlink should succeed physically: {}",
        output.content
    );

    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["content"], "top secret");

    let captured = gate.captured_paths.lock().unwrap().clone();
    assert_eq!(captured.len(), 1, "gate should have been called once");

    let path_str = &captured[0];

    // The gate MUST receive the path that resolves INTO the private directory.
    assert!(
        path_str.contains("private"),
        "The security gate must receive the resolved physical path containing 'private'. Received: {path_str}",
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
