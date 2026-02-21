use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_mcp::{McpClient, ServersConfig};
use astrid_plugins::Plugin;
use astrid_plugins::mcp_plugin::McpPlugin;
use astrid_plugins::{PluginContext, PluginEntryPoint, PluginId, PluginManifest, PluginRegistry};
use astrid_storage::kv::ScopedKvStore;
use astrid_test::{MockLlmTurn, MockToolCall};

use super::super::common::RuntimeTestHarness;

/// Returns `true` when Node.js is available on `$PATH`.
fn node_available() -> bool {
    which::which("node").is_ok()
}

/// Path to the config-echo fixture plugin source.
fn fixture_index_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("astrid-plugins/tests/fixtures/config-echo/index.mjs")
}

/// Write the bridge script and fixture plugin into a temp directory.
fn prepare_bridge_dir(dir: &Path) {
    openclaw_bridge::node_bridge::write_bridge_script(dir).expect("write bridge script");
    let fixture_src = fixture_index_path();
    std::fs::copy(&fixture_src, dir.join("index.mjs")).expect("copy fixture index.mjs");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_plugin_e2e_dispatch() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    let dir = tmp.path();
    prepare_bridge_dir(dir);

    // Provide a config that the mock tool `get-config` will read back
    let mut config = HashMap::new();
    config.insert("network".into(), serde_json::json!("testnet"));
    config.insert("owner".into(), serde_json::json!("0xABC"));

    let manifest = PluginManifest {
        id: PluginId::from_static("config-echo"),
        name: "Config Echo Test Plugin".into(),
        version: "0.1.0".into(),
        description: Some("Test fixture for MCP bridge config delivery".into()),
        author: None,
        entry_point: PluginEntryPoint::Mcp {
            command: "node".into(),
            args: vec![
                "astrid_bridge.mjs".into(),
                "--entry".into(),
                "./index.mjs".into(),
                "--plugin-id".into(),
                "config-echo".into(),
            ],
            env: HashMap::new(),
            binary_hash: None,
        },
        capabilities: vec![],
        connectors: vec![],
        config: config.clone(),
    };

    let mcp_client = McpClient::with_config(ServersConfig::default());
    let mut plugin = McpPlugin::new(manifest, mcp_client).with_plugin_dir(dir.to_path_buf());

    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv = ScopedKvStore::new(store, "plugin:config-echo").expect("create scoped KV store");
    let ctx = PluginContext::new(std::env::temp_dir(), kv, config);

    // Wait for the plugin to start and establish MCP handshakes
    plugin.load(&ctx).await.expect("plugin load");

    // Wait for the async `astrid.setPluginConfig` notification to be processed by Node.
    // We use a generous 500ms timeout here to prevent flakiness on slower CI runners.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let mut registry = PluginRegistry::new();
    registry.register(Box::new(plugin)).unwrap();

    let mut harness = RuntimeTestHarness::with_approval(
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:config-echo:get-config",
                serde_json::json!({}),
            )]),
            MockLlmTurn::text("Config loaded correctly"),
        ],
        ApprovalOption::AllowOnce,
    )
    .with_plugin_registry(registry);

    harness
        .run_turn("trigger the MCP config-echo tool")
        .await
        .unwrap();

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

        let parsed: serde_json::Value =
            serde_json::from_str(&result.content).expect("should return strict JSON output");

        assert_eq!(parsed["network"], "testnet", "network config should match");
        assert_eq!(parsed["owner"], "0xABC", "owner config should match");
    } else {
        panic!("expected ToolResult content");
    }

    let last = harness.session.messages.last().unwrap();
    assert_eq!(last.text(), Some("Config loaded correctly"));
}
