//! End-to-end WASM plugin integration test.
//!
//! Loads the `test-all-endpoints` WASM fixture (compiled from the
//! `test-plugin-guest` Rust crate) and exercises every host function endpoint.
//!
//! The WASM fixture is automatically compiled on first use via
//! `cargo build --target wasm32-unknown-unknown` on the guest crate.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Once};

use astrid_core::plugin_abi::{ToolDefinition, ToolInput, ToolOutput};
use astrid_plugins::PluginId;
use astrid_plugins::wasm::host_functions::register_host_functions;
use astrid_plugins::wasm::host_state::HostState;
use astrid_storage::kv::ScopedKvStore;
use extism::{Manifest, PluginBuilder, UserData, Wasm};

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
    workspace_root: &PathBuf,
    config: HashMap<String, serde_json::Value>,
) -> extism::Plugin {
    let wasm_bytes = std::fs::read(wasm_fixture_path()).expect("read WASM fixture");

    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv =
        ScopedKvStore::new(store, "plugin:test-all-endpoints").expect("create scoped KV store");

    let host_state = HostState {
        plugin_id: PluginId::from_static("test-all-endpoints"),
        workspace_root: workspace_root.clone(),
        kv,
        config,
        security: None,
        runtime_handle: tokio::runtime::Handle::current(),
    };
    let user_data = UserData::new(host_state);

    let extism_wasm = Wasm::data(wasm_bytes);
    let extism_manifest = Manifest::new([extism_wasm]);

    let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
    let builder = register_host_functions(builder, user_data);
    builder.build().expect("build Extism plugin")
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
fn execute_tool(plugin: &mut extism::Plugin, name: &str, args: serde_json::Value) -> ToolOutput {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discover_all_six_tools() {
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

    assert_eq!(
        tools.len(),
        6,
        "expected exactly 6 tools, got {}",
        tools.len()
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
        serde_json::json!({ "message": "hello from e2e" }),
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
        serde_json::json!({ "key": "api_key" }),
    );
    assert!(!output.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["found"], true);
    assert_eq!(parsed["value"], "sk-test-123");

    // Read non-existent key
    let output = execute_tool(
        &mut plugin,
        "test-config",
        serde_json::json!({ "key": "nonexistent" }),
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
        serde_json::json!({ "key": "greeting", "value": "hello world" }),
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
        serde_json::json!({ "path": "test-output.txt", "content": "written by WASM plugin" }),
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
        serde_json::json!({ "path": "test-output.txt" }),
    );
    assert!(!output.is_error, "test-file-read should succeed");
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(parsed["content"], "written by WASM plugin");

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
        serde_json::json!({ "data": test_data }),
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

    let output = execute_tool(&mut plugin, "nonexistent-tool", serde_json::json!({}));

    assert!(output.is_error, "unknown tool should return error");
    assert!(
        output.content.contains("unknown tool"),
        "error should mention unknown tool: {}",
        output.content
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
