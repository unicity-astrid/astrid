//! End-to-end test: generate shim for the `test-all-endpoints` plugin.
//!
//! Verifies that the shim wraps the plugin code correctly and includes
//! all host function bindings needed for the 6 registered tools.
//!
//! The WASM compilation + execution tests require `extism-js` and are gated
//! behind `#[ignore]`. Run with:
//!
//! ```bash
//! cargo test -p openclaw-bridge --test e2e_plugin -- --ignored
//! ```

use std::collections::HashMap;

use openclaw_bridge::shim;

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

    // Host function imports are present
    assert!(
        shim.contains("_astralis_log"),
        "missing astralis_log import"
    );
    assert!(
        shim.contains("_astralis_get_config"),
        "missing astralis_get_config import"
    );
    assert!(
        shim.contains("_astralis_kv_get"),
        "missing astralis_kv_get import"
    );
    assert!(
        shim.contains("_astralis_kv_set"),
        "missing astralis_kv_set import"
    );
    assert!(
        shim.contains("_astralis_read_file"),
        "missing astralis_read_file import"
    );
    assert!(
        shim.contains("_astralis_write_file"),
        "missing astralis_write_file import"
    );
    assert!(
        shim.contains("_astralis_http_request"),
        "missing astralis_http_request import"
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
    assert!(shim.contains("execute-tool"), "missing execute-tool export");
    assert!(shim.contains("run-hook"), "missing run-hook export");

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

/// Compile the shimmed JS to WASM via `extism-js`.
///
/// Requires `extism-js` to be installed. Run with:
/// ```bash
/// cargo test -p openclaw-bridge --test e2e_plugin -- --ignored
/// ```
#[test]
#[ignore = "requires extism-js installed"]
fn compile_test_plugin_to_wasm() {
    // Check extism-js is available
    if which::which("extism-js").is_err() {
        eprintln!("extism-js not found, skipping compilation test");
        return;
    }

    let plugin_code = test_plugin_source();
    let mut config = HashMap::new();
    config.insert("api_key".into(), serde_json::json!("sk-test-123"));

    let shimmed = shim::generate(&plugin_code, &config);

    // Write shimmed JS to temp file
    let tmp_dir = std::env::temp_dir().join("oc-e2e-compile");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let js_path = tmp_dir.join("plugin.js");
    let wasm_path = tmp_dir.join("plugin.wasm");
    std::fs::write(&js_path, &shimmed).expect("write shimmed JS");

    // Compile
    openclaw_bridge::compiler::compile(&js_path, &wasm_path)
        .expect("extism-js compilation should succeed");

    assert!(wasm_path.exists(), "WASM output file should exist");
    let wasm_bytes = std::fs::read(&wasm_path).unwrap();
    assert!(
        wasm_bytes.len() > 1000,
        "WASM file should be non-trivial (got {} bytes)",
        wasm_bytes.len()
    );

    // Copy to the fixture location for the astralis-plugins integration test
    let fixture_dest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("astralis-plugins/tests/fixtures");
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
