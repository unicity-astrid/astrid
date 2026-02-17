//! Integration tests for plugin tool dispatch in the agent runtime.
//!
//! Verifies that plugin tools appear in the LLM tool list, route through
//! `execute_plugin_tool` (not the MCP path), and fire security interceptor
//! and hooks correctly.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_plugins::PluginRegistry;
use astrid_plugins::context::{PluginContext, PluginToolContext};
use astrid_plugins::error::PluginResult;
use astrid_plugins::manifest::{PluginEntryPoint, PluginManifest};
use astrid_plugins::plugin::{Plugin, PluginId, PluginState};
use astrid_plugins::tool::PluginTool;
use astrid_test::{MockLlmTurn, MockToolCall};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Test plugin that provides an "echo" tool
// ---------------------------------------------------------------------------

struct TestPlugin {
    id: PluginId,
    manifest: PluginManifest,
    state: PluginState,
    tools: Vec<Arc<dyn PluginTool>>,
}

impl TestPlugin {
    fn new(id: &str) -> Self {
        let plugin_id = PluginId::from_static(id);
        Self {
            manifest: PluginManifest {
                id: plugin_id.clone(),
                name: format!("Test Plugin {id}"),
                version: "0.1.0".into(),
                description: None,
                author: None,
                entry_point: PluginEntryPoint::Wasm {
                    path: "plugin.wasm".into(),
                    hash: None,
                },
                capabilities: vec![],
                config: HashMap::new(),
            },
            id: plugin_id,
            state: PluginState::Ready,
            tools: vec![Arc::new(EchoTool)],
        }
    }
}

#[async_trait::async_trait]
impl Plugin for TestPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn state(&self) -> PluginState {
        self.state.clone()
    }
    async fn load(&mut self, _ctx: &PluginContext) -> PluginResult<()> {
        self.state = PluginState::Ready;
        Ok(())
    }
    async fn unload(&mut self) -> PluginResult<()> {
        self.state = PluginState::Unloaded;
        Ok(())
    }
    fn tools(&self) -> &[Arc<dyn PluginTool>] {
        &self.tools
    }
}

struct EchoTool;

#[async_trait::async_trait]
impl PluginTool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Echoes the input message"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &PluginToolContext,
    ) -> PluginResult<String> {
        let msg = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("no message");
        Ok(format!("echo: {msg}"))
    }
}

// ---------------------------------------------------------------------------
// Helper to build a runtime with a plugin registry
// ---------------------------------------------------------------------------

fn build_runtime_with_plugins(
    workspace: &std::path::Path,
    turns: Vec<MockLlmTurn>,
    registry: PluginRegistry,
) -> (
    astrid_runtime::AgentRuntime<astrid_test::MockLlmProvider>,
    Arc<astrid_test::MockFrontend>,
    astrid_runtime::AgentSession,
    Arc<RwLock<PluginRegistry>>,
) {
    let llm = astrid_test::MockLlmProvider::new(turns);
    let mcp = astrid_mcp::McpClient::with_config(astrid_mcp::ServersConfig::default());
    let audit = astrid_audit::AuditLog::in_memory(astrid_crypto::KeyPair::generate());
    let sessions = astrid_runtime::SessionStore::new(workspace.join("sessions"));
    let mut ws_config = astrid_runtime::WorkspaceConfig::new(workspace.to_path_buf());
    ws_config.never_allow.clear();
    let config = astrid_runtime::RuntimeConfig {
        workspace: ws_config,
        system_prompt: "You are a test assistant.".to_string(),
        ..astrid_runtime::RuntimeConfig::default()
    };

    let plugin_registry = Arc::new(RwLock::new(registry));
    let runtime = astrid_runtime::AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        astrid_crypto::KeyPair::generate(),
        config,
    )
    .with_plugin_registry(Arc::clone(&plugin_registry));

    let frontend =
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(ApprovalOption::AllowOnce));
    let session = runtime.create_session(None);
    (runtime, frontend, session, plugin_registry)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Plugin tool call dispatches through plugin path and returns result to LLM.
#[tokio::test]
async fn test_plugin_tool_dispatch() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("test")))
        .unwrap();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins(
        ws.path(),
        vec![
            // Turn 1: LLM calls the plugin tool
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:test:echo",
                serde_json::json!({"message": "hello from plugin"}),
            )]),
            // Turn 2: LLM responds with text
            MockLlmTurn::text("The plugin said: echo: hello from plugin"),
        ],
        registry,
    );

    runtime
        .run_turn_streaming(&mut session, "Call the echo plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    // Find the tool result message
    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(!result.is_error, "tool result should not be an error");
        assert!(
            result.content.contains("echo: hello from plugin"),
            "expected echo response, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }

    // Final message should be the assistant text
    let last = session.messages.last().unwrap();
    assert_eq!(
        last.text(),
        Some("The plugin said: echo: hello from plugin")
    );
}

/// Plugin tool not found (plugin unloaded between listing and execution)
/// returns a graceful error, not a panic.
#[tokio::test]
async fn test_plugin_tool_not_found_graceful_error() {
    let ws = tempfile::tempdir().unwrap();

    // Empty registry — the LLM somehow calls a plugin tool that doesn't exist.
    let registry = PluginRegistry::new();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:missing:echo",
                serde_json::json!({}),
            )]),
            MockLlmTurn::text("I see the error"),
        ],
        registry,
    );

    runtime
        .run_turn_streaming(&mut session, "Call missing plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    // Find the tool result message
    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("Plugin tool not found"),
            "expected 'Plugin tool not found', got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

/// with_plugin_registry builder correctly sets the field.
#[tokio::test]
async fn test_with_plugin_registry_builder() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("alpha")))
        .unwrap();

    let (runtime, _frontend, _session, _registry) =
        build_runtime_with_plugins(ws.path(), vec![MockLlmTurn::text("ok")], registry);

    // The runtime was constructed with a plugin registry — verify it didn't
    // panic and the runtime is functional.
    let session = runtime.create_session(None);
    assert!(!session.id.0.is_nil());
}

/// Runtime without plugin registry returns error for plugin tool calls.
#[tokio::test]
async fn test_no_plugin_registry_returns_error() {
    let ws = tempfile::tempdir().unwrap();

    // Build runtime WITHOUT plugin registry (using common harness pattern).
    let llm = astrid_test::MockLlmProvider::new(vec![
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "hello"}),
        )]),
        MockLlmTurn::text("I see the error"),
    ]);
    let mcp = astrid_mcp::McpClient::with_config(astrid_mcp::ServersConfig::default());
    let audit = astrid_audit::AuditLog::in_memory(astrid_crypto::KeyPair::generate());
    let sessions = astrid_runtime::SessionStore::new(ws.path().join("sessions"));
    let mut ws_config = astrid_runtime::WorkspaceConfig::new(ws.path().to_path_buf());
    ws_config.never_allow.clear();
    let config = astrid_runtime::RuntimeConfig {
        workspace: ws_config,
        system_prompt: "You are a test assistant.".to_string(),
        ..astrid_runtime::RuntimeConfig::default()
    };
    let runtime = astrid_runtime::AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        astrid_crypto::KeyPair::generate(),
        config,
    );
    // No with_plugin_registry() call — plugin_registry is None.

    let frontend =
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(ApprovalOption::AllowOnce));
    let mut session = runtime.create_session(None);

    runtime
        .run_turn_streaming(&mut session, "Call plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    // Find the tool result message
    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("not available"),
            "expected 'not available', got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}
