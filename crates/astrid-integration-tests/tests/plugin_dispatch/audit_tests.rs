//! Audit log entry verification tests for plugin tool calls.

use std::sync::Arc;

use astrid_plugins::PluginRegistry;
use astrid_test::{MockLlmTurn, MockToolCall};

use super::fixtures::{FailingTool, TestPlugin, make_plugin};
use super::helpers::build_runtime_with_plugins;

/// After a successful plugin tool call, verify that the audit log contains a
/// `PluginToolCall` entry with the correct `plugin_id` and tool name.
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
