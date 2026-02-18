//! Core plugin tool dispatch tests.
//!
//! Covers happy-path dispatch, graceful not-found errors, registry builder
//! verification, and the no-registry error path.

use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_plugins::PluginRegistry;
use astrid_test::{MockLlmTurn, MockToolCall};

use super::fixtures::TestPlugin;
use super::helpers::build_runtime_with_plugins;

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
