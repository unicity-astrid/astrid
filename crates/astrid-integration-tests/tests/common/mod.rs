//! Shared test harness for integration tests.

use std::sync::Arc;

use astrid_audit::AuditLog;
use astrid_core::ApprovalOption;
use astrid_crypto::KeyPair;
use astrid_mcp::{McpClient, ServersConfig};
use astrid_runtime::{AgentRuntime, AgentSession, RuntimeConfig, SessionStore, WorkspaceConfig};
use astrid_test::{MockFrontend, MockLlmProvider, MockLlmTurn};
use tempfile::TempDir;

/// A self-contained test harness that wires up all runtime components.
///
/// Owns a `TempDir` that acts as the workspace root and session store.
/// The tempdir is cleaned up when the harness is dropped.
#[allow(dead_code)]
pub struct RuntimeTestHarness {
    /// The agent runtime.
    pub runtime: AgentRuntime<MockLlmProvider>,
    /// The mock frontend (shared via Arc for `run_turn_streaming`).
    pub frontend: Arc<MockFrontend>,
    /// A fresh session.
    pub session: AgentSession,
    /// The workspace tempdir (held to prevent cleanup).
    _workspace_dir: TempDir,
}

#[allow(dead_code)]
impl RuntimeTestHarness {
    /// Build a new harness with the given LLM turns and default `AllowOnce` approval.
    pub fn new(turns: Vec<MockLlmTurn>) -> Self {
        Self::with_approval(turns, ApprovalOption::AllowOnce)
    }

    /// Build a new harness with the given LLM turns and a specific default approval.
    pub fn with_approval(turns: Vec<MockLlmTurn>, default_approval: ApprovalOption) -> Self {
        let workspace_dir = TempDir::new().expect("failed to create tempdir");

        let llm = MockLlmProvider::new(turns);
        let mcp = McpClient::with_config(ServersConfig::default());

        let audit_key = KeyPair::generate();
        let audit = AuditLog::in_memory(audit_key);

        let sessions_dir = workspace_dir.path().join("sessions");
        let sessions = SessionStore::new(&sessions_dir);

        let runtime_key = KeyPair::generate();

        let mut ws_config = WorkspaceConfig::new(workspace_dir.path().to_path_buf());
        // Clear never_allow so temp dirs under /var/folders (macOS) aren't treated as protected
        ws_config.never_allow.clear();
        let config = RuntimeConfig {
            workspace: ws_config,
            system_prompt: "You are a test assistant.".to_string(),
            ..RuntimeConfig::default()
        };

        let runtime = AgentRuntime::new(llm, mcp, audit, sessions, runtime_key, config);
        let frontend = Arc::new(MockFrontend::new().with_default_approval(default_approval));
        let session = runtime.create_session(None);

        RuntimeTestHarness {
            runtime,
            frontend,
            session,
            _workspace_dir: workspace_dir,
        }
    }

    /// Attach a plugin registry to the agent runtime.
    pub fn with_plugin_registry(mut self, registry: astrid_plugins::PluginRegistry) -> Self {
        self.runtime = self
            .runtime
            .with_plugin_registry(std::sync::Arc::new(tokio::sync::RwLock::new(registry)));
        self
    }

    /// Attach a pre-wrapped plugin registry Arc to the agent runtime.
    ///
    /// Use when you need to retain a handle for post-test cleanup
    /// (e.g. calling `unload_all()` on MCP plugins).
    pub fn with_plugin_registry_arc(
        mut self,
        registry: std::sync::Arc<tokio::sync::RwLock<astrid_plugins::PluginRegistry>>,
    ) -> Self {
        self.runtime = self.runtime.with_plugin_registry(registry);
        self
    }

    /// Convenience: run a single turn with the given user input.
    pub async fn run_turn(&mut self, input: &str) -> Result<(), astrid_runtime::RuntimeError> {
        self.runtime
            .run_turn_streaming(&mut self.session, input, Arc::clone(&self.frontend))
            .await
    }
}
