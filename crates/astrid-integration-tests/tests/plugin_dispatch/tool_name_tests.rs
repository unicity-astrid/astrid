//! Tests for special characters and edge cases in plugin tool names.

use std::sync::Arc;

use astrid_plugins::PluginRegistry;
use astrid_plugins::context::PluginToolContext;
use astrid_plugins::error::PluginResult;
use astrid_plugins::tool::PluginTool;
use astrid_test::{MockLlmTurn, MockToolCall};

use super::fixtures::make_plugin;
use super::helpers::build_runtime_with_plugins;

/// Invalid plugin ID format (uppercase, spaces) is rejected by `is_plugin_tool`
/// at the routing level, so it never reaches `execute_plugin_tool`.
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
        fn name(&self) -> &'static str {
            "name:with:colons"
        }
        fn description(&self) -> &'static str {
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
