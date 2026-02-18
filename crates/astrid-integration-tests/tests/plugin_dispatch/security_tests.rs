//! Security interceptor and workspace boundary tests.

use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_plugins::PluginRegistry;
use astrid_test::{MockLlmTurn, MockToolCall};
use tokio::sync::RwLock;

use super::fixtures::TestPlugin;
use super::helpers::build_runtime_with_plugins_and_approval;

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
