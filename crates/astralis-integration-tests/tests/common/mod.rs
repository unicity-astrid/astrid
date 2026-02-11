//! Shared test harness for integration tests.

use std::path::PathBuf;
use std::sync::Arc;

use astralis_audit::AuditLog;
use astralis_core::ApprovalOption;
use astralis_crypto::KeyPair;
use astralis_mcp::{McpClient, ServersConfig};
use astralis_runtime::{AgentRuntime, AgentSession, RuntimeConfig, SessionStore, WorkspaceConfig};
use astralis_test::{MockFrontend, MockLlmProvider, MockLlmTurn};
use tempfile::TempDir;

/// A self-contained test harness that wires up all runtime components.
///
/// Owns a `TempDir` that acts as the workspace root and session store.
/// The tempdir is cleaned up when the harness is dropped.
pub struct RuntimeTestHarness {
    /// The agent runtime.
    pub runtime: AgentRuntime<MockLlmProvider>,
    /// The mock frontend (shared via Arc for run_turn_streaming).
    pub frontend: Arc<MockFrontend>,
    /// A fresh session.
    pub session: AgentSession,
    /// The workspace tempdir (held to prevent cleanup).
    #[allow(dead_code)]
    pub workspace_dir: TempDir,
}

impl RuntimeTestHarness {
    /// Build a new harness with the given LLM turns and default AllowOnce approval.
    pub fn new(turns: Vec<MockLlmTurn>) -> Self {
        Self::builder(turns).build()
    }

    /// Start building a harness with customisation options.
    pub fn builder(turns: Vec<MockLlmTurn>) -> HarnessBuilder {
        HarnessBuilder {
            turns,
            default_approval: ApprovalOption::AllowOnce,
            approval_queue: Vec::new(),
        }
    }

    /// Convenience: run a single turn with the given user input.
    pub async fn run_turn(&mut self, input: &str) -> Result<(), astralis_runtime::RuntimeError> {
        self.runtime
            .run_turn_streaming(&mut self.session, input, Arc::clone(&self.frontend))
            .await
    }

    /// Get the workspace root path.
    pub fn workspace_path(&self) -> PathBuf {
        self.workspace_dir.path().to_path_buf()
    }
}

/// Builder for [`RuntimeTestHarness`].
pub struct HarnessBuilder {
    turns: Vec<MockLlmTurn>,
    default_approval: ApprovalOption,
    approval_queue: Vec<ApprovalOption>,
}

impl HarnessBuilder {
    /// Set the default approval option (when queue is empty).
    pub fn default_approval(mut self, option: ApprovalOption) -> Self {
        self.default_approval = option;
        self
    }

    /// Queue specific approval responses.
    pub fn approval_queue(mut self, queue: Vec<ApprovalOption>) -> Self {
        self.approval_queue = queue;
        self
    }

    /// Build the harness.
    pub fn build(self) -> RuntimeTestHarness {
        let workspace_dir = TempDir::new().expect("failed to create tempdir");

        let llm = MockLlmProvider::new(self.turns);

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

        let mut frontend = MockFrontend::new().with_default_approval(self.default_approval);
        for option in self.approval_queue {
            frontend = frontend.with_approval_response(option);
        }
        let frontend = Arc::new(frontend);

        let session = runtime.create_session(None);

        RuntimeTestHarness {
            runtime,
            frontend,
            session,
            workspace_dir,
        }
    }
}
