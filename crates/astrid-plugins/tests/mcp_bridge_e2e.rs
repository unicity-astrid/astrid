//! End-to-end MCP bridge integration tests.
//!
//! Tests the Node.js bridge subprocess using two approaches:
//!
//! 1. **Raw JSON-RPC** — spawns the bridge directly, sends raw messages over
//!    stdin/stdout. Tests the bridge's notification handler in isolation.
//! 2. **[`McpPlugin`] API** — uses the production code path for handshake,
//!    tool discovery, and tool execution.
//!
//! These tests require `node` to be on `$PATH`. They are skipped
//! automatically if Node.js is not available.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use astrid_mcp::{McpClient, ServersConfig};
use astrid_plugins::context::{PluginContext, PluginToolContext};
use astrid_plugins::manifest::{PluginEntryPoint, PluginManifest};
use astrid_plugins::mcp_plugin::McpPlugin;
use astrid_plugins::plugin::{Plugin, PluginId, PluginState};
use astrid_storage::kv::ScopedKvStore;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Path to the config-echo fixture plugin source.
fn fixture_index_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config-echo/index.mjs")
}

/// Returns `true` when Node.js is available on `$PATH`.
fn node_available() -> bool {
    which::which("node").is_ok()
}

/// Write the bridge script and fixture plugin into a temp directory.
fn prepare_bridge_dir(dir: &Path) {
    openclaw_bridge::node_bridge::write_bridge_script(dir).expect("write bridge script");
    let fixture_src = fixture_index_path();
    std::fs::copy(&fixture_src, dir.join("index.mjs")).expect("copy fixture index.mjs");
}

// ---------------------------------------------------------------------------
// McpPlugin-based helpers
// ---------------------------------------------------------------------------

/// Set up a temp directory with the bridge script and the config-echo fixture,
/// returning a plugin ready for `load()`.
fn setup_bridge_plugin(tmp: &tempfile::TempDir) -> McpPlugin {
    let dir = tmp.path();
    prepare_bridge_dir(dir);

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
        config: HashMap::new(),
    };

    let mcp_client = McpClient::with_config(ServersConfig::default());
    McpPlugin::new(manifest, mcp_client).with_plugin_dir(dir.to_path_buf())
}

/// Create a [`PluginContext`] with the given config map.
fn make_context(config: HashMap<String, serde_json::Value>) -> PluginContext {
    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv = ScopedKvStore::new(store, "plugin:config-echo").expect("create scoped KV store");
    PluginContext::new(std::env::temp_dir(), kv, config)
}

/// Create a [`PluginToolContext`] for tool execution.
fn make_tool_context() -> PluginToolContext {
    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv = ScopedKvStore::new(store, "plugin:config-echo").expect("create scoped KV store");
    PluginToolContext::new(
        PluginId::from_static("config-echo"),
        std::env::temp_dir(),
        kv,
    )
}

// ---------------------------------------------------------------------------
// Raw JSON-RPC helper
// ---------------------------------------------------------------------------

/// A lightweight bridge handle that speaks raw JSON-RPC over stdin/stdout.
struct RawBridge {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
}

impl RawBridge {
    /// Spawn the bridge subprocess in the given directory.
    async fn spawn(dir: &Path) -> Self {
        Self::spawn_with(dir, "config-echo").await
    }

    /// Spawn the bridge subprocess with a custom plugin id.
    async fn spawn_with(dir: &Path, plugin_id: &str) -> Self {
        let mut child = tokio::process::Command::new("node")
            .args([
                "astrid_bridge.mjs",
                "--entry",
                "./index.mjs",
                "--plugin-id",
                plugin_id,
            ])
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn bridge");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        Self {
            child,
            stdin,
            reader,
        }
    }

    /// Send a JSON-RPC message (newline-delimited).
    async fn send(&mut self, msg: &serde_json::Value) {
        let line = serde_json::to_string(msg).unwrap();
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write to bridge stdin");
        self.stdin.write_all(b"\n").await.expect("write newline");
        self.stdin.flush().await.expect("flush stdin");
    }

    /// Read the next JSON-RPC response line (with timeout).
    async fn recv(&mut self) -> serde_json::Value {
        let mut line = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            self.reader.read_line(&mut line).await.expect("read line");
        })
        .await
        .expect("bridge response timed out");
        serde_json::from_str(line.trim()).expect("parse JSON-RPC response")
    }

    /// Perform the MCP handshake (initialize + notifications/initialized).
    async fn handshake(&mut self) {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.0.1" }
            }
        }))
        .await;

        let resp = self.recv().await;
        assert_eq!(resp["id"], 0, "initialize response id mismatch");
        assert!(
            resp["result"]["serverInfo"].is_object(),
            "missing serverInfo in initialize response"
        );

        // Send initialized notification (no id, no response expected).
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;
    }

    /// Gracefully shut down the bridge subprocess.
    async fn shutdown(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), self.child.wait()).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that the bridge processes `notifications/astrid.setPluginConfig`
/// and makes the config available via `api.runtime.config.loadConfig()`.
///
/// Uses raw JSON-RPC over stdio to test the notification handler directly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bridge_config_delivery() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    prepare_bridge_dir(tmp.path());

    let mut bridge = RawBridge::spawn(tmp.path()).await;
    bridge.handshake().await;

    // Send the config notification.
    bridge
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/astrid.setPluginConfig",
            "params": {
                "config": {
                    "network": "testnet",
                    "owner": "0xABC"
                }
            }
        }))
        .await;

    // Give the bridge a moment to process the notification (fire-and-forget).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Call get-config tool to read back the config.
    bridge
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get-config", "arguments": {} }
        }))
        .await;

    let resp = bridge.recv().await;
    assert_eq!(resp["id"], 1);

    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("get-config should return text content");
    let config: serde_json::Value = serde_json::from_str(text).expect("parse config JSON");

    assert_eq!(config["network"], "testnet", "config.network mismatch");
    assert_eq!(config["owner"], "0xABC", "config.owner mismatch");

    bridge.shutdown().await;
}

/// Verify that with no config notification, `loadConfig()` returns `{}`.
///
/// Uses raw JSON-RPC to call get-config without sending setPluginConfig.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bridge_empty_config() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    prepare_bridge_dir(tmp.path());

    let mut bridge = RawBridge::spawn(tmp.path()).await;
    bridge.handshake().await;

    // Give the bridge a moment for startServices() to complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    bridge
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get-config", "arguments": {} }
        }))
        .await;

    let resp = bridge.recv().await;
    assert_eq!(resp["id"], 1);

    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("get-config should return text content");
    let config: serde_json::Value = serde_json::from_str(text).expect("parse config JSON");

    assert_eq!(
        config,
        serde_json::json!({}),
        "empty config should return {{}}"
    );

    bridge.shutdown().await;
}

/// Load plugin via [`McpPlugin`] and verify tool discovery: `get-config`
/// and `__astrid_get_agent_context` are both found.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bridge_tool_discovery() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    let mut plugin = setup_bridge_plugin(&tmp);

    let ctx = make_context(HashMap::new());
    plugin.load(&ctx).await.expect("plugin load");
    assert_eq!(plugin.state(), PluginState::Ready);

    let tool_names: Vec<&str> = plugin.tools().iter().map(|t| t.name()).collect();

    assert!(
        tool_names.contains(&"get-config"),
        "should discover get-config tool, found: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"__astrid_get_agent_context"),
        "should discover __astrid_get_agent_context tool, found: {tool_names:?}"
    );
    assert_eq!(
        tool_names.len(),
        2,
        "expected exactly 2 tools, found: {tool_names:?}"
    );

    plugin.unload().await.expect("plugin unload");
}

/// Load plugin via [`McpPlugin`] with config, call `get-config`, verify
/// the config notification was delivered and `loadConfig()` returns it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bridge_config_via_plugin() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    let mut plugin = setup_bridge_plugin(&tmp);

    let mut config = HashMap::new();
    config.insert("network".into(), serde_json::json!("testnet"));
    config.insert("owner".into(), serde_json::json!("0xABC"));

    let ctx = make_context(config);
    plugin.load(&ctx).await.expect("plugin load");

    // Wait for the notification to be processed by the bridge.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let tool = plugin
        .tools()
        .iter()
        .find(|t| t.name() == "get-config")
        .expect("get-config tool should be registered");

    let tool_ctx = make_tool_context();
    let result = tool
        .execute(serde_json::json!({}), &tool_ctx)
        .await
        .expect("get-config execution");

    let parsed: serde_json::Value =
        serde_json::from_str(&result).expect("parse get-config result as JSON");

    assert_eq!(parsed["network"], "testnet", "config.network mismatch");
    assert_eq!(parsed["owner"], "0xABC", "config.owner mismatch");

    plugin.unload().await.expect("plugin unload");
}

/// Load plugin via [`McpPlugin`] (no config), call `get-config`, verify
/// tool execution returns `{}`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bridge_tool_execution() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    let mut plugin = setup_bridge_plugin(&tmp);

    let ctx = make_context(HashMap::new());
    plugin.load(&ctx).await.expect("plugin load");

    // Wait for services to be ready.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let tool = plugin
        .tools()
        .iter()
        .find(|t| t.name() == "get-config")
        .expect("get-config tool should be registered");

    let tool_ctx = make_tool_context();
    let result = tool
        .execute(serde_json::json!({}), &tool_ctx)
        .await
        .expect("get-config execution");

    let parsed: serde_json::Value =
        serde_json::from_str(&result).expect("parse get-config result as JSON");

    assert_eq!(
        parsed,
        serde_json::json!({}),
        "no-config plugin should return {{}}"
    );

    plugin.unload().await.expect("plugin unload");
}

// ---------------------------------------------------------------------------
// Channel-echo fixture helpers
// ---------------------------------------------------------------------------

/// Path to the channel-echo fixture plugin source.
fn channel_echo_index_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/channel-echo/index.mjs")
}

/// Write the bridge script and channel-echo fixture into a temp directory.
fn prepare_channel_echo_dir(dir: &Path) {
    openclaw_bridge::node_bridge::write_bridge_script(dir).expect("write bridge script");
    let fixture_src = channel_echo_index_path();
    std::fs::copy(&fixture_src, dir.join("index.mjs")).expect("copy channel-echo index.mjs");
}

/// Set up a temp directory with the bridge script and the channel-echo fixture,
/// returning a plugin ready for `load()`.
fn setup_channel_echo_plugin(tmp: &tempfile::TempDir) -> McpPlugin {
    let dir = tmp.path();
    prepare_channel_echo_dir(dir);

    let manifest = PluginManifest {
        id: PluginId::from_static("channel-echo"),
        name: "Channel Echo Test Plugin".into(),
        version: "0.1.0".into(),
        description: Some("Test fixture for connector registration".into()),
        author: None,
        entry_point: PluginEntryPoint::Mcp {
            command: "node".into(),
            args: vec![
                "astrid_bridge.mjs".into(),
                "--entry".into(),
                "./index.mjs".into(),
                "--plugin-id".into(),
                "channel-echo".into(),
            ],
            env: HashMap::new(),
            binary_hash: None,
        },
        capabilities: vec![],
        connectors: vec![],
        config: HashMap::new(),
    };

    let mcp_client = McpClient::with_config(ServersConfig::default());
    McpPlugin::new(manifest, mcp_client).with_plugin_dir(dir.to_path_buf())
}

// ---------------------------------------------------------------------------
// Connector registration tests
// ---------------------------------------------------------------------------

/// Raw JSON-RPC test: verify the bridge sends a `connectorRegistered`
/// notification after receiving `notifications/initialized`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bridge_connector_registered_notification() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    prepare_channel_echo_dir(tmp.path());

    let mut bridge = RawBridge::spawn_with(tmp.path(), "channel-echo").await;

    // Send initialize request.
    bridge
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.0.1" }
            }
        }))
        .await;

    let resp = bridge.recv().await;
    assert_eq!(resp["id"], 0, "initialize response id mismatch");

    // Send initialized notification — this should trigger the batch connectorRegistered.
    bridge
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;

    // The next line on stdout should be the connectorRegistered notification.
    let notification = bridge.recv().await;

    assert_eq!(
        notification["method"], "notifications/astrid.connectorRegistered",
        "expected connectorRegistered notification, got: {notification}"
    );
    assert_eq!(
        notification["params"]["pluginId"], "channel-echo",
        "pluginId mismatch"
    );

    let channels = notification["params"]["channels"]
        .as_array()
        .expect("channels should be an array");
    assert_eq!(channels.len(), 1, "expected 1 channel");
    assert_eq!(channels[0]["name"], "telegram", "channel name mismatch");

    bridge.shutdown().await;
}

/// `McpPlugin` test: verify connectors are populated after `load()`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_plugin_connector_registration() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    let mut plugin = setup_channel_echo_plugin(&tmp);

    let store = Arc::new(astrid_storage::MemoryKvStore::new());
    let kv = ScopedKvStore::new(store, "plugin:channel-echo").expect("create scoped KV store");
    let ctx = PluginContext::new(std::env::temp_dir(), kv, HashMap::new());

    plugin.load(&ctx).await.expect("plugin load");
    assert_eq!(plugin.state(), PluginState::Ready);

    // The bridge sends connectorRegistered right after initialized.
    // refresh_connectors() drains pending notices from the mpsc channel.
    // The rmcp dispatch loop may not have forwarded the notification yet,
    // so poll briefly until it arrives.
    let mut found = false;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if !plugin.refresh_connectors().is_empty() {
            found = true;
            break;
        }
    }
    assert!(found, "expected at least one connector after polling");

    let telegram = plugin
        .connectors()
        .iter()
        .find(|c| c.name == "telegram")
        .expect("should have a 'telegram' connector")
        .clone();

    assert_eq!(
        telegram.frontend_type,
        astrid_core::identity::FrontendType::Telegram,
        "frontend_type mismatch"
    );

    assert!(
        matches!(
            &telegram.source,
            astrid_core::connector::ConnectorSource::OpenClaw { plugin_id } if plugin_id == "channel-echo"
        ),
        "source mismatch: {:?}",
        telegram.source
    );

    // The fixture declares canReceive + canSend only (least privilege).
    let expected_caps = astrid_core::connector::ConnectorCapabilities {
        can_receive: true,
        can_send: true,
        can_approve: false,
        can_elicit: false,
        supports_rich_media: false,
        supports_threads: false,
        supports_buttons: false,
    };
    assert_eq!(
        telegram.capabilities, expected_caps,
        "capabilities should match what the fixture declared, not full()"
    );

    plugin.unload().await.expect("plugin unload");
}

// ---------------------------------------------------------------------------
// Hook context fixture helpers & tests
// ---------------------------------------------------------------------------

/// Path to the hook-context fixture plugin source.
fn hook_context_index_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hook-context/index.mjs")
}

/// Write the bridge script and hook-context fixture into a temp directory.
fn prepare_hook_context_dir(dir: &Path) {
    openclaw_bridge::node_bridge::write_bridge_script(dir).expect("write bridge script");
    let fixture_src = hook_context_index_path();
    std::fs::copy(&fixture_src, dir.join("index.mjs")).expect("copy hook-context index.mjs");
}

/// Raw JSON-RPC test: verify the bridge correctly translates `notifications/astrid.hookEvent`
/// to a plugin `api.on("session_start")` handler, and its effect can be observed via tool execution.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bridge_hook_context_delivery() {
    if !node_available() {
        eprintln!("SKIP: node not found on $PATH");
        return;
    }

    let tmp = tempfile::tempdir().expect("create temp dir");
    prepare_hook_context_dir(tmp.path());

    let mut bridge = RawBridge::spawn_with(tmp.path(), "hook-context").await;
    bridge.handshake().await;

    // Send the hookEvent notification for 'session_start'.
    bridge
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/astrid.hookEvent",
            "params": {
                "event": "session_start",
                "data": null
            }
        }))
        .await;

    // Give the bridge a moment to process the async event handler.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Call get-hook-state to verify state mutation.
    bridge
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get-hook-state", "arguments": {} }
        }))
        .await;

    let resp = bridge.recv().await;
    assert_eq!(resp["id"], 1);

    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("should return text context");

    assert_eq!(
        text, "Injected system identity context.",
        "context was not mutated efficiently by hook event"
    );

    bridge.shutdown().await;
}
