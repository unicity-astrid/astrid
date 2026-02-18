//! Runtime builder helpers for plugin dispatch integration tests.

use std::sync::Arc;

use astrid_core::ApprovalOption;
use astrid_plugins::PluginRegistry;
use astrid_test::MockLlmTurn;
use tokio::sync::RwLock;

pub fn build_runtime_with_plugins(
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

pub fn build_runtime_with_plugins_and_approval(
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
