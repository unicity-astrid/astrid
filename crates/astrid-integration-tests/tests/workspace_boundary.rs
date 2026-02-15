//! Integration tests for workspace boundary enforcement.

mod common;

use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_test::{MockLlmTurn, MockToolCall};

/// Helper: build a runtime with a specific workspace directory and LLM turns.
/// The `never_allow` list is cleared so temp dirs under `/var/folders` (macOS) aren't blocked.
fn build_runtime_with_workspace(
    workspace: &std::path::Path,
    turns: Vec<MockLlmTurn>,
    default_approval: ApprovalOption,
) -> (
    astrid_runtime::AgentRuntime<astrid_test::MockLlmProvider>,
    Arc<astrid_test::MockFrontend>,
    astrid_runtime::AgentSession,
) {
    let llm = astrid_test::MockLlmProvider::new(turns);
    let mcp = astrid_mcp::McpClient::with_config(astrid_mcp::ServersConfig::default());
    let audit = astrid_audit::AuditLog::in_memory(astrid_crypto::KeyPair::generate());
    let sessions = astrid_runtime::SessionStore::new(workspace.join("sessions"));
    let mut ws_config = astrid_runtime::WorkspaceConfig::new(workspace.to_path_buf());
    // Clear never_allow so temp dirs under /var/folders (macOS) aren't treated as protected
    ws_config.never_allow.clear();
    let config = astrid_runtime::RuntimeConfig {
        workspace: ws_config,
        system_prompt: "Test".to_string(),
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
    let frontend =
        Arc::new(astrid_test::MockFrontend::new().with_default_approval(default_approval));
    let session = runtime.create_session(None);
    (runtime, frontend, session)
}

#[tokio::test]
async fn test_path_inside_workspace_no_escape_check() {
    let ws = tempfile::tempdir().unwrap();
    let test_file = ws.path().join("test.txt");
    std::fs::write(&test_file, "inside workspace").unwrap();
    let file_path = test_file.to_str().unwrap().to_string();

    let (runtime, frontend, mut session) = build_runtime_with_workspace(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "read_file",
                serde_json::json!({"file_path": file_path}),
            )]),
            MockLlmTurn::text("got it"),
        ],
        ApprovalOption::AllowOnce,
    );

    runtime
        .run_turn_streaming(&mut session, "Read the file", Arc::clone(&frontend))
        .await
        .unwrap();

    let last = session.messages.last().unwrap();
    assert_eq!(last.text(), Some("got it"));
}

#[tokio::test]
async fn test_path_outside_workspace_denied() {
    // Create a file outside the workspace
    let outside_dir = tempfile::tempdir().unwrap();
    let outside_file = outside_dir.path().join("outside.txt");
    std::fs::write(&outside_file, "secret data").unwrap();
    let outside_path = outside_file.to_str().unwrap().to_string();

    let ws = tempfile::tempdir().unwrap();
    let (runtime, frontend, mut session) = build_runtime_with_workspace(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "read_file",
                serde_json::json!({"file_path": outside_path}),
            )]),
            MockLlmTurn::text("ok"),
        ],
        // Default deny — will deny the workspace escape
        ApprovalOption::Deny,
    );

    runtime
        .run_turn_streaming(&mut session, "Read outside file", Arc::clone(&frontend))
        .await
        .unwrap();

    // The tool result should be an error (denied)
    let tool_result_msg = session
        .messages
        .iter()
        .find(|m| matches!(m.role, astrid_llm::MessageRole::Tool));
    assert!(
        tool_result_msg.is_some(),
        "should have a tool result message"
    );

    if let astrid_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("denied") || result.content.contains("outside workspace"),
            "expected denial message, got: {}",
            result.content
        );
    }
}

#[tokio::test]
async fn test_path_outside_workspace_approved() {
    // Create a file outside the workspace
    let outside_dir = tempfile::tempdir().unwrap();
    let outside_file = outside_dir.path().join("outside.txt");
    std::fs::write(&outside_file, "secret data").unwrap();
    let outside_path = outside_file.to_str().unwrap().to_string();

    let ws = tempfile::tempdir().unwrap();
    let (runtime, frontend, mut session) = build_runtime_with_workspace(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "read_file",
                serde_json::json!({"file_path": outside_path.clone()}),
            )]),
            MockLlmTurn::text("I read it"),
        ],
        // AllowSession — will approve and record in escape handler
        ApprovalOption::AllowSession,
    );

    runtime
        .run_turn_streaming(&mut session, "Read the outside file", Arc::clone(&frontend))
        .await
        .unwrap();

    // Should succeed — path was approved
    let last = session.messages.last().unwrap();
    assert_eq!(last.text(), Some("I read it"));

    // The escape handler should have recorded the approval
    assert!(
        session
            .escape_handler
            .is_allowed(&std::path::PathBuf::from(&outside_path)),
        "escape handler should record approved path"
    );
}
