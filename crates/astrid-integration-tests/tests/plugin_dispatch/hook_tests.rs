//! Hook interaction tests: `PreToolCall` blocking, `PostToolCall` and `ToolError` firing.

use std::collections::HashMap;
use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_plugins::PluginRegistry;
use astrid_test::{MockLlmTurn, MockToolCall};
use tokio::sync::RwLock;

use super::fixtures::{FailingTool, TestPlugin, make_plugin};

/// A `PreToolCall` hook that outputs "block: <reason>" prevents the plugin tool
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

/// After a successful plugin tool call, the `PostToolCall` hook fires (not `ToolError`).
/// We verify this by checking that a `PostToolCall` command hook ran successfully —
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

/// After a failing plugin tool call, the `ToolError` hook fires (not `PostToolCall`).
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
