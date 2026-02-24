//! End-to-end test: generate shim for the `test-all-endpoints` plugin.
//!
//! Verifies that the shim wraps the plugin code correctly and includes
//! all host function bindings needed for the 6 registered tools.
//!
//! The WASM compilation test requires the `QuickJS` kernel to be built.
//! If the kernel is a placeholder stub, the compile test is skipped.

use std::collections::HashMap;

use openclaw_bridge::shim;
use openclaw_bridge::transpiler;

/// Read the test plugin source.
fn test_plugin_source() -> String {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/test-all-endpoints/index.js");
    std::fs::read_to_string(fixture).expect("test-all-endpoints/index.js should exist")
}

#[test]
fn shim_wraps_test_plugin_with_all_host_functions() {
    let plugin_code = test_plugin_source();
    let mut config = HashMap::new();
    config.insert("api_key".into(), serde_json::json!("sk-test-123"));
    config.insert("debug".into(), serde_json::json!(true));

    let shim = shim::generate(&plugin_code, &config);

    // Host function references are present (lazy-resolved via _getHostFn)
    assert!(
        shim.contains("\"astrid_log\""),
        "missing astrid_log reference"
    );
    assert!(
        shim.contains("\"astrid_get_config\""),
        "missing astrid_get_config reference"
    );
    assert!(
        shim.contains("\"astrid_kv_get\""),
        "missing astrid_kv_get reference"
    );
    assert!(
        shim.contains("\"astrid_kv_set\""),
        "missing astrid_kv_set reference"
    );
    assert!(
        shim.contains("\"astrid_read_file\""),
        "missing astrid_read_file reference"
    );
    assert!(
        shim.contains("\"astrid_write_file\""),
        "missing astrid_write_file reference"
    );
    assert!(
        shim.contains("\"astrid_http_request\""),
        "missing astrid_http_request reference"
    );

    // Host function wrappers are present
    assert!(
        shim.contains("function hostLog("),
        "missing hostLog wrapper"
    );
    assert!(
        shim.contains("function hostKvGet("),
        "missing hostKvGet wrapper"
    );
    assert!(
        shim.contains("function hostKvSet("),
        "missing hostKvSet wrapper"
    );
    assert!(
        shim.contains("function hostReadFile("),
        "missing hostReadFile wrapper"
    );
    assert!(
        shim.contains("function hostWriteFile("),
        "missing hostWriteFile wrapper"
    );
    assert!(
        shim.contains("function hostGetConfig("),
        "missing hostGetConfig wrapper"
    );
    assert!(
        shim.contains("function hostHttpRequest("),
        "missing hostHttpRequest wrapper"
    );

    // Config keys are baked in
    assert!(shim.contains("\"api_key\""), "missing api_key config key");
    assert!(shim.contains("\"debug\""), "missing debug config key");

    // Plugin code is embedded
    assert!(
        shim.contains("test-all-endpoints activating"),
        "plugin code not embedded"
    );
    assert!(
        shim.contains("registerTool"),
        "registerTool calls not present"
    );

    // Extism exports are present
    assert!(
        shim.contains("describe-tools"),
        "missing describe-tools export"
    );
    assert!(shim.contains("astrid_tool_call"), "missing astrid_tool_call export");
    assert!(shim.contains("astrid_hook_trigger"), "missing astrid_hook_trigger export");

    // OpenClaw context mock is present
    assert!(
        shim.contains("_openclawContext"),
        "missing OpenClaw context mock"
    );
    assert!(
        shim.contains("logger"),
        "missing logger in OpenClaw context"
    );
}

#[test]
fn shim_registers_all_six_tools() {
    let plugin_code = test_plugin_source();
    let config = HashMap::new();
    let shim = shim::generate(&plugin_code, &config);

    // All 6 tool registrations should be in the shimmed code
    assert!(
        shim.contains("\"test-log\""),
        "test-log tool not registered"
    );
    assert!(
        shim.contains("\"test-config\""),
        "test-config tool not registered"
    );
    assert!(shim.contains("\"test-kv\""), "test-kv tool not registered");
    assert!(
        shim.contains("\"test-file-write\""),
        "test-file-write tool not registered"
    );
    assert!(
        shim.contains("\"test-file-read\""),
        "test-file-read tool not registered"
    );
    assert!(
        shim.contains("\"test-roundtrip\""),
        "test-roundtrip tool not registered"
    );
}

#[test]
fn transpile_js_fixture_passthrough() {
    // Plain JS should pass through the transpiler cleanly
    let source = test_plugin_source();
    let result = transpiler::transpile(&source, "index.js").unwrap();
    // The transpiled output should still contain the key plugin patterns
    assert!(
        result.contains("registerTool"),
        "registerTool should survive transpilation"
    );
    assert!(
        result.contains("test-log"),
        "tool names should survive transpilation"
    );
}

/// Compile the shimmed JS to WASM via the embedded Wizer + kernel.
///
/// This test requires the real `QuickJS` kernel (not the placeholder stub).
/// It is skipped if the kernel is too small to be real.
#[test]
fn compile_test_plugin_to_wasm() {
    // Check if the kernel is a real build (not the placeholder stub)
    let kernel_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("kernel/engine.wasm");
    let kernel_size = std::fs::metadata(&kernel_path)
        .map(|m| m.len())
        .unwrap_or(0);

    if kernel_size < 100 {
        eprintln!(
            "QuickJS kernel is a placeholder ({kernel_size} bytes), skipping compile test. \
             Build the real kernel — see kernel/README.md"
        );
        return;
    }

    let plugin_code = test_plugin_source();
    let mut config = HashMap::new();
    config.insert("api_key".into(), serde_json::json!("sk-test-123"));

    let shimmed = shim::generate(&plugin_code, &config);

    // Write shimmed JS to temp file for debugging
    let tmp_dir = std::env::temp_dir().join("oc-e2e-compile");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let wasm_path = tmp_dir.join("plugin.wasm");

    // Compile (now takes string, not path)
    openclaw_bridge::compiler::compile(&shimmed, &wasm_path)
        .expect("embedded compilation should succeed");

    assert!(wasm_path.exists(), "WASM output file should exist");
    let wasm_bytes = std::fs::read(&wasm_path).unwrap();
    assert!(
        wasm_bytes.len() > 1000,
        "WASM file should be non-trivial (got {} bytes)",
        wasm_bytes.len()
    );

    // Verify WASM magic
    assert_eq!(&wasm_bytes[..4], b"\0asm", "output should be valid WASM");

    // Copy to the fixture location for the astrid-plugins integration test
    let fixture_dest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("astrid-plugins/tests/fixtures");
    let _ = std::fs::create_dir_all(&fixture_dest);
    std::fs::copy(&wasm_path, fixture_dest.join("test-all-endpoints.wasm"))
        .expect("copy WASM fixture");

    eprintln!(
        "Compiled test plugin: {} bytes -> {}",
        wasm_bytes.len(),
        fixture_dest.join("test-all-endpoints.wasm").display()
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

/// Test transpiling a `TypeScript` plugin source.
#[test]
fn transpile_typescript_plugin() {
    let ts_source = r#"
interface ToolDef {
    description: string;
    inputSchema: object;
}

function activate(ctx: any): void {
    ctx.logger.info("TypeScript plugin activating");
    ctx.registerTool("ts-test", {
        description: "A TypeScript test tool",
        inputSchema: { type: "object" }
    } as ToolDef, (name: string, args: Record<string, unknown>) => {
        return "hello from TypeScript";
    });
}

module.exports = { activate };
"#;

    let result = transpiler::transpile(ts_source, "plugin.ts").unwrap();
    // Type annotations should be stripped
    assert!(!result.contains(": any"), "should strip type annotations");
    assert!(!result.contains(": void"), "should strip return type");
    assert!(
        !result.contains("interface ToolDef"),
        "should strip interfaces"
    );
    assert!(
        !result.contains("as ToolDef"),
        "should strip type assertions"
    );
    // Core logic should be preserved
    assert!(
        result.contains("registerTool"),
        "should preserve registerTool call"
    );
    assert!(result.contains("ts-test"), "should preserve tool name");
}
