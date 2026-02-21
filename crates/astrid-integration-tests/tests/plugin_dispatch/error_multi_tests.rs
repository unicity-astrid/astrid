//! Plugin tool error propagation and multi-plugin dispatch tests.

use std::sync::Arc;

use astrid_plugins::PluginRegistry;
use astrid_test::{MockLlmTurn, MockToolCall};

use super::fixtures::{FailingTool, TestPlugin, UpperTool, make_plugin};
use super::helpers::build_runtime_with_plugins;

/// Plugin tool that returns Err(...) from `execute()` produces a `ToolCallResult`
/// with `is_error` = true and the error message in content.
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
