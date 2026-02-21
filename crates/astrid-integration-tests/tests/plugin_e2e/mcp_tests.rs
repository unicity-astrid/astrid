use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_mcp::{McpClient, ServersConfig};
use astrid_plugins::Plugin;
use astrid_plugins::mcp_plugin::McpPlugin;
use astrid_plugins::{
    PluginContext, PluginEntryPoint, PluginId, PluginManifest, PluginRegistry, PluginToolContext,
};
use astrid_storage::kv::ScopedKvStore;
use astrid_test::{MockLlmTurn, MockToolCall};

use super::super::common::RuntimeTestHarness;

/// Returns `true` when Node.js is available on `$PATH`.
fn node_available() -> bool {
    which::which("node").is_ok()
}

/// Path to the config-echo fixture plugin source.
///
/// This depends on the fixture at `astrid-plugins/tests/fixtures/config-echo/index.mjs`.
/// If that file is moved or renamed, this path must be updated.
fn fixture_index_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR always has a parent")
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
        println!("SKIP: node not found on $PATH — skipping MCP E2E test");
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
    let kv =
        ScopedKvStore::new(store.clone(), "plugin:config-echo").expect("create scoped KV store");
    let ctx = PluginContext::new(dir.to_path_buf(), kv, config.clone());

    // Load the plugin (spawns Node.js subprocess + MCP handshake)
    plugin.load(&ctx).await.expect("plugin load");

    // Poll until the async `astrid.setPluginConfig` notification has been processed
    // and the `get-config` tool returns config with the "network" key present.
    let tool = plugin
        .tools()
        .iter()
        .find(|t| t.name() == "get-config")
        .expect("get-config tool should be discovered after load")
        .clone();

    let tool_kv =
        ScopedKvStore::new(store, "plugin:config-echo").expect("create tool scoped KV store");
    let tool_ctx = PluginToolContext::new(
        PluginId::from_static("config-echo"),
        dir.to_path_buf(),
        tool_kv,
    )
    .with_config(config.clone());

    let mut config_ready = false;
    for _ in 0..20 {
        if let Ok(result) = tool.execute(serde_json::json!({}), &tool_ctx).await {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                if parsed.get("network").is_some() {
                    config_ready = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        config_ready,
        "config-echo plugin did not receive config within 1s"
    );

    let mut registry = PluginRegistry::new();
    registry.register(Box::new(plugin)).unwrap();

    let registry = Arc::new(tokio::sync::RwLock::new(registry));

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
    .with_plugin_registry_arc(Arc::clone(&registry));

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

    // Gracefully shut down the Node.js subprocess
    registry.write().await.unload_all().await;
}
