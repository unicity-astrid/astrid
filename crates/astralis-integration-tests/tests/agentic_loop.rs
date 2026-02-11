//! Integration tests for the full agentic loop.

mod common;

use std::sync::Arc;

use astralis_core::ApprovalOption;
use astralis_test::{MockLlmTurn, MockToolCall};
use common::RuntimeTestHarness;

/// Helper: build a runtime with a specific workspace directory and LLM turns.
fn build_runtime_with_workspace(
    workspace: &std::path::Path,
    turns: Vec<MockLlmTurn>,
    default_approval: ApprovalOption,
) -> (
    astralis_runtime::AgentRuntime<astralis_test::MockLlmProvider>,
    Arc<astralis_test::MockFrontend>,
    astralis_runtime::AgentSession,
) {
    let llm = astralis_test::MockLlmProvider::new(turns);
    let mcp = astralis_mcp::McpClient::with_config(astralis_mcp::ServersConfig::default());
    let audit = astralis_audit::AuditLog::in_memory(astralis_crypto::KeyPair::generate());
    let sessions = astralis_runtime::SessionStore::new(workspace.join("sessions"));
    let mut ws_config = astralis_runtime::WorkspaceConfig::new(workspace.to_path_buf());
    // Clear never_allow so temp dirs under /var/folders (macOS) aren't treated as protected
    ws_config.never_allow.clear();
    let config = astralis_runtime::RuntimeConfig {
        workspace: ws_config,
        system_prompt: "You are a test assistant.".to_string(),
        ..astralis_runtime::RuntimeConfig::default()
    };
    let runtime = astralis_runtime::AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        astralis_crypto::KeyPair::generate(),
        config,
    );
    let frontend =
        Arc::new(astralis_test::MockFrontend::new().with_default_approval(default_approval));
    let session = runtime.create_session(None);
    (runtime, frontend, session)
}

// ---------------------------------------------------------------------------
// B1. Full chat turn
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_turn_text_only() {
    let mut harness = RuntimeTestHarness::new(vec![MockLlmTurn::text("Hello! How can I help?")]);

    harness.run_turn("Hello").await.unwrap();

    // User message + assistant text = 2 messages
    assert_eq!(harness.session.messages.len(), 2);

    let last = harness.session.messages.last().unwrap();
    assert_eq!(last.text(), Some("Hello! How can I help?"));
}

#[tokio::test]
async fn test_full_turn_with_tool_call() {
    // Create workspace dir first, then write file, then create runtime
    let ws = tempfile::tempdir().unwrap();
    let test_file = ws.path().join("test.txt");
    std::fs::write(&test_file, "hello world").unwrap();
    let file_path = test_file.to_str().unwrap().to_string();

    let (runtime, frontend, mut session) = build_runtime_with_workspace(
        ws.path(),
        vec![
            // Turn 1: LLM calls read_file
            MockLlmTurn::tool_calls(vec![MockToolCall::new(
                "read_file",
                serde_json::json!({"file_path": file_path}),
            )]),
            // Turn 2: LLM responds with text
            MockLlmTurn::text("The file says: hello world"),
        ],
        ApprovalOption::AllowOnce,
    );

    runtime
        .run_turn_streaming(&mut session, "Read test.txt", Arc::clone(&frontend))
        .await
        .unwrap();

    // user, assistant+tools, tool_result, assistant+text = 4 messages
    assert!(
        session.messages.len() >= 4,
        "expected >= 4 messages, got {}",
        session.messages.len()
    );

    let last = session.messages.last().unwrap();
    assert_eq!(last.text(), Some("The file says: hello world"));
}

#[tokio::test]
async fn test_multi_tool_turn() {
    let ws = tempfile::tempdir().unwrap();
    let a_file = ws.path().join("a.txt");
    let b_file = ws.path().join("b.txt");
    std::fs::write(&a_file, "content a").unwrap();
    std::fs::write(&b_file, "content b").unwrap();
    let a_path = a_file.to_str().unwrap().to_string();
    let b_path = b_file.to_str().unwrap().to_string();

    let (runtime, frontend, mut session) = build_runtime_with_workspace(
        ws.path(),
        vec![
            MockLlmTurn::tool_calls(vec![
                MockToolCall::new("read_file", serde_json::json!({"file_path": a_path})),
                MockToolCall::new("read_file", serde_json::json!({"file_path": b_path})),
            ]),
            MockLlmTurn::text("Both files read"),
        ],
        ApprovalOption::AllowOnce,
    );

    runtime
        .run_turn_streaming(&mut session, "Read both files", Arc::clone(&frontend))
        .await
        .unwrap();

    // Count tool result messages
    let tool_results: Vec<_> = session
        .messages
        .iter()
        .filter(|m| matches!(m.role, astralis_llm::MessageRole::Tool))
        .collect();
    assert_eq!(
        tool_results.len(),
        2,
        "should have 2 tool results, got {}",
        tool_results.len()
    );

    let last = session.messages.last().unwrap();
    assert_eq!(last.text(), Some("Both files read"));
}

// ---------------------------------------------------------------------------
// B3–B5. Error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_llm_error_propagates() {
    let mut harness = RuntimeTestHarness::new(vec![MockLlmTurn::error("API unavailable")]);

    let result = harness.run_turn("Hello").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("API unavailable")
            || err.to_string().contains("Stream")
            || err.to_string().contains("error"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn test_unknown_builtin_tool() {
    let mut harness = RuntimeTestHarness::builder(vec![
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "nonexistent_tool",
            serde_json::json!({}),
        )]),
        MockLlmTurn::text("I see the error"),
    ])
    .default_approval(ApprovalOption::AllowOnce)
    .build();

    harness.run_turn("Do something").await.unwrap();

    // Find the tool result message
    let tool_result_msg = harness
        .session
        .messages
        .iter()
        .find(|m| matches!(m.role, astralis_llm::MessageRole::Tool));
    assert!(tool_result_msg.is_some(), "should have a tool result");

    if let astralis_llm::MessageContent::ToolResult(ref result) = tool_result_msg.unwrap().content {
        assert!(result.is_error, "tool result should be an error");
        assert!(
            result.content.contains("Unknown built-in tool"),
            "expected 'Unknown built-in tool', got: {}",
            result.content
        );
    } else {
        panic!("expected ToolResult content");
    }
}

#[tokio::test]
async fn test_malformed_tool_args_returns_error_to_llm() {
    let mut harness = RuntimeTestHarness::builder(vec![
        MockLlmTurn::tool_calls(vec![MockToolCall::new(
            "read_file",
            serde_json::json!({"wrong_field": "val"}),
        )]),
        MockLlmTurn::text("I see the error"),
    ])
    .default_approval(ApprovalOption::AllowOnce)
    .build();

    // This should complete successfully — the error is returned as a tool result,
    // not as a fatal error.
    harness.run_turn("Read something").await.unwrap();

    // Should have completed with messages
    assert!(
        harness.session.messages.len() >= 3,
        "expected >= 3 messages, got {}",
        harness.session.messages.len()
    );
}
