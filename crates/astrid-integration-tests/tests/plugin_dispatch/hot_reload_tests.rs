//! Hot-reload mid-turn race condition tests.
//!
//! Simulates plugins being unloaded or transitioning to Failed state between
//! the time the LLM sees the tool list and when the runtime tries to execute
//! the tool call.

use std::sync::Arc;

use astrid_plugins::PluginRegistry;
use astrid_plugins::plugin::{PluginId, PluginState};
use astrid_test::{MockLlmTurn, MockToolCall};

use super::fixtures::TestPlugin;
use super::helpers::build_runtime_with_plugins;

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
