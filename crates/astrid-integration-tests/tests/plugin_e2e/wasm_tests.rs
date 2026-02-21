use astrid_core::ApprovalOption;
use astrid_plugins::Plugin;
use astrid_plugins::wasm::WasmPluginLoader;
use astrid_plugins::{PluginContext, PluginEntryPoint, PluginId, PluginManifest, PluginRegistry};
use astrid_test::{MockLlmTurn, MockToolCall};
use std::path::PathBuf;
use std::sync::{Arc, Once};

use super::super::common::RuntimeTestHarness;

static BUILD_FIXTURE: Once = Once::new();

/// Build the `test-plugin-guest` crate to WASM and copy the artifact into
/// the integration-tests fixtures directory so the test does not depend on
/// a cross-crate build output path at runtime.
fn build_fixture() {
    BUILD_FIXTURE.call_once(|| {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_dir = manifest_dir
            .parent()
            .expect("CARGO_MANIFEST_DIR always has a parent");
        let guest_dir = workspace_dir.join("test-plugin-guest");

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

        // Copy the compiled .wasm into the integration-tests fixtures directory
        let src = workspace_dir
            .join("test-plugin-guest/target/wasm32-unknown-unknown/release/test_plugin_guest.wasm");
        let fixtures_dir = manifest_dir.join("tests/fixtures");
        std::fs::create_dir_all(&fixtures_dir).expect("create fixtures dir");
        std::fs::copy(&src, fixtures_dir.join("test_plugin_guest.wasm"))
            .expect("copy wasm fixture");
    });
}

fn wasm_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_plugin_guest.wasm")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_wasm_plugin_e2e_dispatch() {
    build_fixture();

    let manifest = PluginManifest {
        id: PluginId::from_static("test-all-endpoints"),
        name: "Test All Endpoints".into(),
        version: "0.1.0".into(),
        description: None,
        author: None,
        entry_point: PluginEntryPoint::Wasm {
            path: wasm_fixture_path(),
            hash: None,
        },
        capabilities: vec![],
        connectors: vec![],
        config: std::collections::HashMap::new(),
    };

    let loader = WasmPluginLoader::new()
        .with_timeout(std::time::Duration::from_secs(30))
        .with_require_hash(false);
    let mut plugin = loader.create_plugin(manifest);

    // Use a local tempdir for the workspace root so cleanup is deterministic.
    // Note: the runtime internally resolves its own workspace root, so this
    // path is only used by the plugin context during load.
    let plugin_tmp = tempfile::tempdir().expect("create temp dir for WASM plugin context");

    let kv_store = Arc::new(astrid_storage::MemoryKvStore::new());
    let scoped_kv =
        astrid_storage::kv::ScopedKvStore::new(kv_store, "plugin:test-all-endpoints").unwrap();
    let ctx = PluginContext::new(
        plugin_tmp.path().to_path_buf(),
        scoped_kv,
        std::collections::HashMap::new(),
    );
    plugin.load(&ctx).await.unwrap();

    let mut registry = PluginRegistry::new();
    registry.register(Box::new(plugin)).unwrap();

    let mut harness = RuntimeTestHarness::with_approval(
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:test-all-endpoints:test-log",
                serde_json::json!({"message": "hello from inside wasm E2E"}),
            )]),
            MockLlmTurn::text("WASM tool executed successfully"),
        ],
        ApprovalOption::AllowOnce,
    )
    .with_plugin_registry(registry);

    harness.run_turn("trigger the WASM log tool").await.unwrap();

    // Verify the tool was called and returned success
    let tool_result_msg = harness
        .session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));

    assert!(tool_result_msg.is_some(), "should have a tool result");
    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(
            !result.is_error,
            "tool result should not be an error: {}",
            result.content
        );
        assert!(
            result.content.contains("hello from inside wasm E2E"),
            "expected echo response, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }

    let last = harness.session.messages.last().unwrap();
    assert_eq!(last.text(), Some("WASM tool executed successfully"));
}
