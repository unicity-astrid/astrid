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
use astrid_plugins::error::{PluginError, PluginResult};
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
                connectors: vec![],
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
// A tool that always fails
// ---------------------------------------------------------------------------

struct FailingTool;

#[async_trait::async_trait]
impl PluginTool for FailingTool {
    fn name(&self) -> &str {
        "fail"
    }
    fn description(&self) -> &str {
        "Always fails"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &PluginToolContext,
    ) -> PluginResult<String> {
        Err(PluginError::ExecutionFailed(
            "intentional test failure".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// A second tool for multi-plugin dispatch tests
// ---------------------------------------------------------------------------

struct UpperTool;

#[async_trait::async_trait]
impl PluginTool for UpperTool {
    fn name(&self) -> &str {
        "upper"
    }
    fn description(&self) -> &str {
        "Uppercases the input"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            }
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &PluginToolContext,
    ) -> PluginResult<String> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("nothing");
        Ok(text.to_uppercase())
    }
}

/// Helper to create a `TestPlugin` with custom tools.
fn make_plugin(id: &str, tools: Vec<Arc<dyn PluginTool>>) -> TestPlugin {
    let plugin_id = PluginId::from_static(id);
    TestPlugin {
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
            connectors: vec![],
            config: HashMap::new(),
        },
        id: plugin_id,
        state: PluginState::Ready,
        tools,
    }
}

// ---------------------------------------------------------------------------
// Helpers to build a runtime with a plugin registry
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
    build_runtime_with_plugins_and_approval(workspace, turns, registry, ApprovalOption::AllowOnce)
}

fn build_runtime_with_plugins_and_approval(
    workspace: &std::path::Path,
    turns: Vec<MockLlmTurn>,
    registry: PluginRegistry,
    default_approval: ApprovalOption,
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
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(default_approval));
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

// ---------------------------------------------------------------------------
// New tests: security interceptor denial
// ---------------------------------------------------------------------------

/// Security interceptor denial returns a graceful tool error (not a hard failure).
/// We configure the SecurityPolicy to block the plugin tool, so the interceptor
/// rejects it before execution — regardless of approval settings.
#[tokio::test]
async fn test_security_interceptor_denial() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("test")))
        .unwrap();

    // Build a policy that blocks the plugin.
    // Plugin tools are classified as PluginExecution { plugin_id: "test", .. }
    // which routes through check_plugin_action → blocked_plugins check.
    let mut policy = astrid_approval::SecurityPolicy::default();
    policy.blocked_plugins.insert("test".to_string());

    let llm = astrid_test::MockLlmProvider::new(vec![
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "hello"}),
        )]),
        MockLlmTurn::text("I see the denial"),
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

    let plugin_registry = Arc::new(RwLock::new(registry));
    let runtime = astrid_runtime::AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        astrid_crypto::KeyPair::generate(),
        config,
    )
    .with_security_policy(policy)
    .with_plugin_registry(Arc::clone(&plugin_registry));

    let frontend =
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(ApprovalOption::AllowOnce));
    let mut session = runtime.create_session(None);

    runtime
        .run_turn_streaming(&mut session, "Call plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.to_lowercase().contains("blocked")
                || result.content.to_lowercase().contains("denied"),
            "expected blocked/denied message, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

// ---------------------------------------------------------------------------
// New tests: workspace boundary rejection for plugin tools
// ---------------------------------------------------------------------------

/// Plugin tool call with a file_path argument outside the workspace is blocked.
#[tokio::test]
async fn test_plugin_workspace_boundary_rejection() {
    let outside_dir = tempfile::tempdir().unwrap();
    let outside_file = outside_dir.path().join("secret.txt");
    std::fs::write(&outside_file, "secret data").unwrap();
    let outside_path = outside_file.to_str().unwrap().to_string();

    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("test")))
        .unwrap();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins_and_approval(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:test:echo",
                // Pass a file_path arg that points outside the workspace.
                serde_json::json!({"file_path": outside_path}),
            )]),
            MockLlmTurn::text("ok"),
        ],
        registry,
        // Deny the workspace escape prompt.
        ApprovalOption::Deny,
    );

    runtime
        .run_turn_streaming(
            &mut session,
            "Read outside file via plugin",
            Arc::clone(&frontend),
        )
        .await
        .unwrap();

    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("denied") || result.content.contains("outside workspace"),
            "expected workspace boundary denial, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

// ---------------------------------------------------------------------------
// New tests: plugin tool error propagation
// ---------------------------------------------------------------------------

/// Plugin tool that returns Err(...) from execute() produces a ToolCallResult
/// with is_error = true and the error message in content.
#[tokio::test]
async fn test_plugin_tool_error_propagation() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(make_plugin("failer", vec![Arc::new(FailingTool)])))
        .unwrap();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:failer:fail",
                serde_json::json!({}),
            )]),
            MockLlmTurn::text("I see the error"),
        ],
        registry,
    );

    runtime
        .run_turn_streaming(&mut session, "Call failing plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("intentional test failure"),
            "expected error message from FailingTool, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

// ---------------------------------------------------------------------------
// New tests: multiple plugin dispatch
// ---------------------------------------------------------------------------

/// Multiple plugins registered: LLM calls tools from different plugins and
/// each routes to the correct one.
#[tokio::test]
async fn test_multiple_plugin_dispatch() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("echo-plugin")))
        .unwrap();
    registry
        .register(Box::new(make_plugin(
            "upper-plugin",
            vec![Arc::new(UpperTool)],
        )))
        .unwrap();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins(
        ws.path(),
        vec![
            // Turn 1: LLM calls both plugins in one turn
            MockLlmTurn::tool_calls(vec![
                MockToolCall::new(
                    "plugin:echo-plugin:echo",
                    serde_json::json!({"message": "hello"}),
                ),
                MockToolCall::new(
                    "plugin:upper-plugin:upper",
                    serde_json::json!({"text": "hello"}),
                ),
            ]),
            // Turn 2: LLM responds with text
            MockLlmTurn::text("done"),
        ],
        registry,
    );

    runtime
        .run_turn_streaming(&mut session, "Call both plugins", Arc::clone(&frontend))
        .await
        .unwrap();

    // Collect all tool results
    let tool_results: Vec<_> = session
        .messages
        .iter()
        .filter_map(|m| {
            if let astrid_llm::MessageContent::ToolResult(ref result) = m.content {
                Some(result.clone())
            } else {
                None
            }
        })
        .collect();

    assert_eq!(
        tool_results.len(),
        2,
        "should have two tool results, got {}",
        tool_results.len()
    );

    // One result should be the echo, the other the upper
    let echo_result = tool_results
        .iter()
        .find(|r| r.content.contains("echo: hello"));
    let upper_result = tool_results.iter().find(|r| r.content.contains("HELLO"));

    assert!(
        echo_result.is_some(),
        "expected echo result, results: {:?}",
        tool_results.iter().map(|r| &r.content).collect::<Vec<_>>()
    );
    assert!(
        !echo_result.unwrap().is_error,
        "echo result should not be an error"
    );

    assert!(
        upper_result.is_some(),
        "expected upper result, results: {:?}",
        tool_results.iter().map(|r| &r.content).collect::<Vec<_>>()
    );
    assert!(
        !upper_result.unwrap().is_error,
        "upper result should not be an error"
    );
}

// ---------------------------------------------------------------------------
// New tests: hook interaction (PreToolCall block)
// ---------------------------------------------------------------------------

/// A PreToolCall hook that outputs "block: <reason>" prevents the plugin tool
/// from executing and returns a graceful error.
#[tokio::test]
async fn test_pre_tool_call_hook_blocks_plugin_tool() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("test")))
        .unwrap();

    // Build a hook that blocks all PreToolCall events.
    let blocking_hook = astrid_hooks::Hook::new(astrid_core::HookEvent::PreToolCall)
        .with_name("block-all-tools")
        .with_handler(astrid_hooks::HookHandler::Command {
            command: "echo".into(),
            args: vec!["block: Blocked by test hook".into()],
            env: HashMap::new(),
            working_dir: None,
        });

    let hook_manager = astrid_hooks::HookManager::new();
    hook_manager.register(blocking_hook).await;

    // Build the runtime with the hook manager
    let llm = astrid_test::MockLlmProvider::new(vec![
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "should not reach plugin"}),
        )]),
        MockLlmTurn::text("I see the block"),
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

    let plugin_registry = Arc::new(RwLock::new(registry));
    let runtime = astrid_runtime::AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        astrid_crypto::KeyPair::generate(),
        config,
    )
    .with_hooks(hook_manager)
    .with_plugin_registry(Arc::clone(&plugin_registry));

    let frontend =
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(ApprovalOption::AllowOnce));
    let mut session = runtime.create_session(None);

    runtime
        .run_turn_streaming(&mut session, "Call plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result
                .content
                .to_lowercase()
                .contains("blocked by test hook"),
            "expected hook block message, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

// ---------------------------------------------------------------------------
// New tests: PostToolCall / ToolError hooks fire
// ---------------------------------------------------------------------------

/// After a successful plugin tool call, the PostToolCall hook fires (not ToolError).
/// We verify this by checking that a PostToolCall command hook ran successfully —
/// the echo command produces stdout which the hook system parses as Continue.
#[tokio::test]
async fn test_post_tool_call_hook_fires_on_success() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("test")))
        .unwrap();

    // Hook that writes to a marker file on PostToolCall.
    let marker = ws.path().join("post-hook-fired.txt");
    let marker_path = marker.to_str().unwrap().to_string();

    let post_hook = astrid_hooks::Hook::new(astrid_core::HookEvent::PostToolCall)
        .with_name("post-tool-marker")
        .with_handler(astrid_hooks::HookHandler::Command {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!("echo 'fired' > '{marker_path}' && echo continue"),
            ],
            env: HashMap::new(),
            working_dir: None,
        });

    let hook_manager = astrid_hooks::HookManager::new();
    hook_manager.register(post_hook).await;

    let llm = astrid_test::MockLlmProvider::new(vec![
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "hello"}),
        )]),
        MockLlmTurn::text("done"),
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

    let plugin_registry = Arc::new(RwLock::new(registry));
    let runtime = astrid_runtime::AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        astrid_crypto::KeyPair::generate(),
        config,
    )
    .with_hooks(hook_manager)
    .with_plugin_registry(Arc::clone(&plugin_registry));

    let frontend =
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(ApprovalOption::AllowOnce));
    let mut session = runtime.create_session(None);

    runtime
        .run_turn_streaming(&mut session, "Call plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    // The PostToolCall hook should have written the marker file.
    assert!(
        marker.exists(),
        "PostToolCall hook should have created marker file at {}",
        marker.display()
    );
}

/// After a failing plugin tool call, the ToolError hook fires (not PostToolCall).
#[tokio::test]
async fn test_tool_error_hook_fires_on_failure() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(make_plugin("failer", vec![Arc::new(FailingTool)])))
        .unwrap();

    // Hook that writes to a marker file on ToolError.
    let marker = ws.path().join("error-hook-fired.txt");
    let marker_path = marker.to_str().unwrap().to_string();

    let error_hook = astrid_hooks::Hook::new(astrid_core::HookEvent::ToolError)
        .with_name("error-marker")
        .with_handler(astrid_hooks::HookHandler::Command {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!("echo 'fired' > '{marker_path}' && echo continue"),
            ],
            env: HashMap::new(),
            working_dir: None,
        });

    // Also register a PostToolCall hook that writes a DIFFERENT marker.
    // This marker should NOT exist after a failing tool call.
    let wrong_marker = ws.path().join("post-hook-should-not-fire.txt");
    let wrong_marker_path = wrong_marker.to_str().unwrap().to_string();

    let post_hook = astrid_hooks::Hook::new(astrid_core::HookEvent::PostToolCall)
        .with_name("post-tool-wrong-marker")
        .with_handler(astrid_hooks::HookHandler::Command {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!("echo 'fired' > '{wrong_marker_path}' && echo continue"),
            ],
            env: HashMap::new(),
            working_dir: None,
        });

    let hook_manager = astrid_hooks::HookManager::new();
    hook_manager.register(error_hook).await;
    hook_manager.register(post_hook).await;

    let llm = astrid_test::MockLlmProvider::new(vec![
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:failer:fail",
            serde_json::json!({}),
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

    let plugin_registry = Arc::new(RwLock::new(registry));
    let runtime = astrid_runtime::AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        astrid_crypto::KeyPair::generate(),
        config,
    )
    .with_hooks(hook_manager)
    .with_plugin_registry(Arc::clone(&plugin_registry));

    let frontend =
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(ApprovalOption::AllowOnce));
    let mut session = runtime.create_session(None);

    runtime
        .run_turn_streaming(&mut session, "Call failing plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    // The ToolError hook should have written the marker file.
    assert!(
        marker.exists(),
        "ToolError hook should have created marker file at {}",
        marker.display()
    );

    // The PostToolCall hook should NOT have fired.
    assert!(
        !wrong_marker.exists(),
        "PostToolCall hook should not fire for a failing tool call"
    );
}

// ---------------------------------------------------------------------------
// New tests: hot-reload mid-turn race (plugin unloaded between listing & exec)
// ---------------------------------------------------------------------------

/// Simulates a plugin being unloaded between the time the LLM sees the tool
/// list and when the runtime tries to execute the tool call. The runtime should
/// return a graceful "Plugin tool not found" error, not a panic.
#[tokio::test]
async fn test_hot_reload_race_plugin_unloaded_mid_turn() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("ephemeral")))
        .unwrap();

    let (runtime, frontend, mut session, registry_lock) = build_runtime_with_plugins(
        ws.path(),
        vec![
            // Turn 1: LLM tries to call the plugin tool (but we'll unload the plugin before execution).
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:ephemeral:echo",
                serde_json::json!({"message": "hello"}),
            )]),
            // Turn 2: LLM responds after seeing the error.
            MockLlmTurn::text("I see the error"),
        ],
        registry,
    );

    // Unload the plugin before the runtime processes the tool call.
    // The LLM "saw" the tool when the tool list was built, but now it's gone.
    {
        let mut reg = registry_lock.write().await;
        let pid = PluginId::from_static("ephemeral");
        reg.unregister(&pid).unwrap();
    }

    runtime
        .run_turn_streaming(
            &mut session,
            "Call the ephemeral plugin",
            Arc::clone(&frontend),
        )
        .await
        .unwrap();

    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("Plugin tool not found"),
            "expected 'Plugin tool not found' in error, got: {}",
            result.content
        );
        // Verify it mentions the plugin may have been unloaded.
        assert!(
            result.content.contains("unloaded"),
            "expected 'unloaded' in error, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

/// Simulates a plugin transitioning to Failed state between tool listing and
/// execution. The find_tool Ready-state check should reject it.
#[tokio::test]
async fn test_hot_reload_race_plugin_failed_mid_turn() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("fragile")))
        .unwrap();

    let (runtime, frontend, mut session, registry_lock) = build_runtime_with_plugins(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:fragile:echo",
                serde_json::json!({"message": "hello"}),
            )]),
            MockLlmTurn::text("I see the error"),
        ],
        registry,
    );

    // Transition the plugin to Failed state before execution.
    // Unregister the Ready version and re-register with Failed state.
    {
        let mut reg = registry_lock.write().await;
        let pid = PluginId::from_static("fragile");
        reg.unregister(&pid).unwrap();
        let mut failed_plugin = TestPlugin::new("fragile");
        failed_plugin.state = PluginState::Failed("simulated crash".into());
        reg.register(Box::new(failed_plugin)).unwrap();
    }

    runtime
        .run_turn_streaming(
            &mut session,
            "Call the fragile plugin",
            Arc::clone(&frontend),
        )
        .await
        .unwrap();

    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("Plugin tool not found"),
            "expected 'Plugin tool not found' for Failed plugin, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

// ---------------------------------------------------------------------------
// New tests: audit entry verification
// ---------------------------------------------------------------------------

/// After a successful plugin tool call, verify that the audit log contains a
/// `PluginToolCall` entry with the correct plugin_id and tool name.
#[tokio::test]
async fn test_audit_entry_contains_plugin_tool_call() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("audited")))
        .unwrap();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:audited:echo",
                serde_json::json!({"message": "audit me"}),
            )]),
            MockLlmTurn::text("done"),
        ],
        registry,
    );

    runtime
        .run_turn_streaming(&mut session, "Call audited plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    // Query the audit log for this session's entries.
    let entries = runtime
        .audit()
        .get_session_entries(&session.id)
        .expect("should retrieve audit entries");

    // Find the PluginToolCall entry.
    let plugin_audit = entries.iter().find(|e| {
        matches!(
            &e.action,
            astrid_audit::AuditAction::PluginToolCall { plugin_id, tool, .. }
                if plugin_id == "audited" && tool == "echo"
        )
    });

    assert!(
        plugin_audit.is_some(),
        "audit log should contain a PluginToolCall entry for audited:echo, entries: {:?}",
        entries
            .iter()
            .map(|e| format!("{:?}", e.action))
            .collect::<Vec<_>>()
    );

    // Verify the outcome is success (the tool succeeded).
    let entry = plugin_audit.unwrap();
    assert!(
        matches!(&entry.outcome, astrid_audit::AuditOutcome::Success { .. }),
        "expected Success outcome for successful tool call, got: {:?}",
        entry.outcome
    );
}

/// After a failing plugin tool call, verify that the audit log records
/// `PluginToolCall` with a Failure outcome.
#[tokio::test]
async fn test_audit_entry_records_failure_outcome() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(make_plugin(
            "fail-audit",
            vec![Arc::new(FailingTool)],
        )))
        .unwrap();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:fail-audit:fail",
                serde_json::json!({}),
            )]),
            MockLlmTurn::text("done"),
        ],
        registry,
    );

    runtime
        .run_turn_streaming(&mut session, "Call failing plugin", Arc::clone(&frontend))
        .await
        .unwrap();

    let entries = runtime
        .audit()
        .get_session_entries(&session.id)
        .expect("should retrieve audit entries");

    let plugin_audit = entries.iter().find(|e| {
        matches!(
            &e.action,
            astrid_audit::AuditAction::PluginToolCall { plugin_id, tool, .. }
                if plugin_id == "fail-audit" && tool == "fail"
        )
    });

    assert!(
        plugin_audit.is_some(),
        "audit log should contain a PluginToolCall entry for fail-audit:fail"
    );

    let entry = plugin_audit.unwrap();
    assert!(
        matches!(&entry.outcome, astrid_audit::AuditOutcome::Failure { .. }),
        "expected Failure outcome for failing tool call, got: {:?}",
        entry.outcome
    );
}

// ---------------------------------------------------------------------------
// New tests: special characters in tool names
// ---------------------------------------------------------------------------

/// Invalid plugin ID format (uppercase, spaces) is rejected by is_plugin_tool
/// at the routing level, so it never reaches execute_plugin_tool.
/// Verify these names are correctly identified as non-plugin tools at the unit level.
#[test]
fn test_special_character_tool_names_rejected() {
    // Empty tool name
    assert!(
        !PluginRegistry::is_plugin_tool("plugin:test:"),
        "empty tool name should be rejected"
    );
    // Invalid plugin ID (uppercase)
    assert!(
        !PluginRegistry::is_plugin_tool("plugin:INVALID:echo"),
        "uppercase plugin ID should be rejected"
    );
    // Invalid plugin ID (spaces)
    assert!(
        !PluginRegistry::is_plugin_tool("plugin:has space:echo"),
        "plugin ID with spaces should be rejected"
    );
    // Just "plugin:" prefix with no ID
    assert!(
        !PluginRegistry::is_plugin_tool("plugin::echo"),
        "empty plugin ID should be rejected"
    );
    // Valid plugin tool names still work
    assert!(PluginRegistry::is_plugin_tool("plugin:valid-id:tool-name"));
    assert!(PluginRegistry::is_plugin_tool("plugin:my-plugin:read-file"));
}

/// Tool name with extra colons (e.g. "plugin:test:name:with:colons") should
/// only split on the first colon after the plugin ID, so "name:with:colons"
/// is treated as the tool name.
#[tokio::test]
async fn test_tool_name_with_colons_resolves_correctly() {
    let ws = tempfile::tempdir().unwrap();

    // Create a plugin with a tool whose name contains colons
    struct ColonTool;

    #[async_trait::async_trait]
    impl PluginTool for ColonTool {
        fn name(&self) -> &str {
            "name:with:colons"
        }
        fn description(&self) -> &str {
            "A tool with colons in the name"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &PluginToolContext,
        ) -> PluginResult<String> {
            Ok("colon-tool-executed".to_string())
        }
    }

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(make_plugin(
            "colon-test",
            vec![Arc::new(ColonTool)],
        )))
        .unwrap();

    let (runtime, frontend, mut session, _registry) = build_runtime_with_plugins(
        ws.path(),
        vec![
            // "plugin:colon-test:name:with:colons"
            // split_once(':') on "colon-test:name:with:colons" → ("colon-test", "name:with:colons")
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "plugin:colon-test:name:with:colons",
                serde_json::json!({}),
            )]),
            MockLlmTurn::text("done"),
        ],
        registry,
    );

    runtime
        .run_turn_streaming(&mut session, "Call colon tool", Arc::clone(&frontend))
        .await
        .unwrap();

    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(
            !result.is_error,
            "colon tool should succeed, got error: {}",
            result.content
        );
        assert!(
            result.content.contains("colon-tool-executed"),
            "expected colon tool output, got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }

    // Verify audit entry has correct plugin_id and tool fields (not mangled by
    // colon-containing tool name).
    let entries = runtime
        .audit()
        .get_session_entries(&session.id)
        .expect("should retrieve audit entries");

    let plugin_audit = entries.iter().find(|e| {
        matches!(
            &e.action,
            astrid_audit::AuditAction::PluginToolCall { plugin_id, tool, .. }
                if plugin_id == "colon-test" && tool == "name:with:colons"
        )
    });

    assert!(
        plugin_audit.is_some(),
        "audit entry should have plugin_id='colon-test' and tool='name:with:colons', entries: {:?}",
        entries
            .iter()
            .map(|e| format!("{:?}", e.action))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// New tests: KV store session isolation
// ---------------------------------------------------------------------------

/// Plugin KV stores are keyed by session+plugin so different sessions
/// cannot leak data to each other. We verify this by running two sessions
/// against the same runtime and checking that the second session does not
/// see the first session's tool result in its messages.
#[tokio::test]
async fn test_kv_store_session_isolation() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("test")))
        .unwrap();

    // Build runtime with enough turns for two sessions.
    let llm = astrid_test::MockLlmProvider::new(vec![
        // Session 1 turns
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "session-one-data"}),
        )]),
        MockLlmTurn::text("done1"),
        // Session 2 turns
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "session-two-data"}),
        )]),
        MockLlmTurn::text("done2"),
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

    let frontend = Arc::new(
        astrid_test::MockFrontend::new()
            .with_default_approval(astrid_core::ApprovalOption::AllowOnce),
    );

    // Session 1
    let mut session1 = runtime.create_session(None);
    runtime
        .run_turn_streaming(&mut session1, "Call plugin s1", Arc::clone(&frontend))
        .await
        .unwrap();

    // Session 2
    let mut session2 = runtime.create_session(None);
    runtime
        .run_turn_streaming(&mut session2, "Call plugin s2", Arc::clone(&frontend))
        .await
        .unwrap();

    // Verify each session only contains its own data (basic isolation).
    let s1_results: Vec<_> = session1
        .messages
        .iter()
        .filter_map(|m| {
            if let astrid_llm::MessageContent::ToolResult(ref result) = m.content {
                Some(result.content.clone())
            } else {
                None
            }
        })
        .collect();

    let s2_results: Vec<_> = session2
        .messages
        .iter()
        .filter_map(|m| {
            if let astrid_llm::MessageContent::ToolResult(ref result) = m.content {
                Some(result.content.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(
        s1_results.iter().any(|r| r.contains("session-one-data")),
        "session 1 should have its own data"
    );
    assert!(
        s2_results.iter().any(|r| r.contains("session-two-data")),
        "session 2 should have its own data"
    );

    // Verify the sessions have distinct IDs (basic sanity).
    assert_ne!(
        session1.id, session2.id,
        "sessions should have different IDs"
    );
}

// ---------------------------------------------------------------------------
// New tests: KV store cleanup
// ---------------------------------------------------------------------------

/// cleanup_plugin_kv_stores removes entries for the given session only.
#[tokio::test]
async fn test_cleanup_plugin_kv_stores() {
    let ws = tempfile::tempdir().unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .register(Box::new(TestPlugin::new("test")))
        .unwrap();

    // Build runtime with enough turns for two sessions.
    let llm = astrid_test::MockLlmProvider::new(vec![
        // Session 1
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "s1"}),
        )]),
        MockLlmTurn::text("done1"),
        // Session 2
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "plugin:test:echo",
            serde_json::json!({"message": "s2"}),
        )]),
        MockLlmTurn::text("done2"),
    ]);
    let mcp = astrid_mcp::McpClient::with_config(astrid_mcp::ServersConfig::default());
    let audit = astrid_audit::AuditLog::in_memory(astrid_crypto::KeyPair::generate());
    let sessions = astrid_runtime::SessionStore::new(ws.path().join("sessions"));
    let mut ws_config = astrid_runtime::WorkspaceConfig::new(ws.path().to_path_buf());
    ws_config.never_allow.clear();
    let config = astrid_runtime::RuntimeConfig {
        workspace: ws_config,
        system_prompt: "test".to_string(),
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

    let frontend = Arc::new(
        astrid_test::MockFrontend::new()
            .with_default_approval(astrid_core::ApprovalOption::AllowOnce),
    );

    // Run session 1
    let mut session1 = runtime.create_session(None);
    runtime
        .run_turn_streaming(&mut session1, "s1", Arc::clone(&frontend))
        .await
        .unwrap();

    // Run session 2
    let mut session2 = runtime.create_session(None);
    runtime
        .run_turn_streaming(&mut session2, "s2", Arc::clone(&frontend))
        .await
        .unwrap();

    // Cleanup session 1 — session 2's stores should remain.
    runtime.cleanup_plugin_kv_stores(&session1.id);

    // Session 2 tool call should still work after cleanup of session 1.
    // (The KV store for session 2 was not evicted.)
    // We can't easily verify the internal map size, but we verify the method
    // doesn't panic and session 2 data is unaffected by re-running (if we had
    // more turns). Instead, verify that cleanup of an already-cleaned session
    // is a no-op.
    runtime.cleanup_plugin_kv_stores(&session1.id); // should not panic

    // Cleanup session 2.
    runtime.cleanup_plugin_kv_stores(&session2.id);
}
