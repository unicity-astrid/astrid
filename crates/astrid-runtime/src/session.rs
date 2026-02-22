//! Agent session management.
//!
//! Sessions track conversation state, capabilities, and context.

use astrid_approval::allowance::Allowance;
use astrid_approval::budget::{BudgetSnapshot, BudgetTracker, WorkspaceBudgetTracker};
use astrid_approval::{AllowanceStore, ApprovalManager, DeferredResolutionStore};
use astrid_capabilities::CapabilityStore;
use astrid_core::SessionId;
use astrid_llm::Message;
use astrid_workspace::escape::{EscapeHandler, EscapeState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// An agent session.
#[derive(Debug)]
pub struct AgentSession {
    /// Unique session identifier.
    pub id: SessionId,
    /// User identifier (key ID).
    pub user_id: [u8; 8],
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Session capabilities.
    pub capabilities: Arc<CapabilityStore>,
    /// Session allowance store.
    pub allowance_store: Arc<AllowanceStore>,
    /// Session approval manager.
    pub approval_manager: Arc<ApprovalManager>,
    /// System prompt.
    pub system_prompt: String,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// Estimated token count.
    pub token_count: usize,
    /// Session metadata.
    pub metadata: SessionMetadata,
    /// Workspace escape handler for tracking allowed paths.
    pub escape_handler: EscapeHandler,
    /// Per-session budget tracker.
    pub budget_tracker: Arc<BudgetTracker>,
    /// Workspace cumulative budget tracker (shared across sessions).
    pub workspace_budget_tracker: Option<Arc<WorkspaceBudgetTracker>>,
    /// Workspace path this session belongs to (for workspace-scoped listing).
    pub workspace_path: Option<PathBuf>,
    /// Model used for this session (e.g. `"claude-sonnet-4-20250514"`).
    pub model: Option<String>,
    /// Whether this session belongs to a sub-agent (skip spark preamble in `run_loop`).
    pub is_subagent: bool,
    /// Plugin-provided context (fetched dynamically per subagent/session, not persisted).
    pub plugin_context: Option<String>,
}

impl AgentSession {
    /// Create a new session.
    #[must_use]
    pub fn new(user_id: [u8; 8], system_prompt: impl Into<String>) -> Self {
        let allowance_store = Arc::new(AllowanceStore::new());
        let deferred_queue = Arc::new(DeferredResolutionStore::new());
        let approval_manager = Arc::new(ApprovalManager::new(
            Arc::clone(&allowance_store),
            deferred_queue,
        ));
        Self {
            id: SessionId::new(),
            user_id,
            messages: Vec::new(),
            capabilities: Arc::new(CapabilityStore::in_memory()),
            allowance_store,
            approval_manager,
            system_prompt: system_prompt.into(),
            created_at: Utc::now(),
            token_count: 0,
            metadata: SessionMetadata::default(),
            escape_handler: EscapeHandler::new(),
            budget_tracker: Arc::new(BudgetTracker::default()),
            workspace_budget_tracker: None,
            workspace_path: None,
            model: None,
            is_subagent: false,
            plugin_context: None,
        }
    }

    /// Create with a specific session ID.
    #[must_use]
    pub fn with_id(id: SessionId, user_id: [u8; 8], system_prompt: impl Into<String>) -> Self {
        let allowance_store = Arc::new(AllowanceStore::new());
        let deferred_queue = Arc::new(DeferredResolutionStore::new());
        let approval_manager = Arc::new(ApprovalManager::new(
            Arc::clone(&allowance_store),
            deferred_queue,
        ));
        Self {
            id,
            user_id,
            messages: Vec::new(),
            capabilities: Arc::new(CapabilityStore::in_memory()),
            allowance_store,
            approval_manager,
            system_prompt: system_prompt.into(),
            created_at: Utc::now(),
            token_count: 0,
            metadata: SessionMetadata::default(),
            escape_handler: EscapeHandler::new(),
            budget_tracker: Arc::new(BudgetTracker::default()),
            workspace_budget_tracker: None,
            workspace_path: None,
            model: None,
            is_subagent: false,
            plugin_context: None,
        }
    }

    /// Create a child session that shares parent's stores.
    ///
    /// The child inherits the parent's `AllowanceStore`, `CapabilityStore`, and
    /// `BudgetTracker` (same `Arc` — spend is visible bidirectionally). The
    /// `ApprovalManager` and `DeferredResolutionStore` are fresh (independent
    /// handler registration and independent deferred queue).
    #[must_use]
    pub fn with_shared_stores(
        id: SessionId,
        user_id: [u8; 8],
        system_prompt: impl Into<String>,
        allowance_store: Arc<AllowanceStore>,
        capabilities: Arc<CapabilityStore>,
        budget_tracker: Arc<BudgetTracker>,
    ) -> Self {
        let deferred_queue = Arc::new(DeferredResolutionStore::new());
        let approval_manager = Arc::new(ApprovalManager::new(
            Arc::clone(&allowance_store),
            deferred_queue,
        ));
        Self {
            id,
            user_id,
            messages: Vec::new(),
            capabilities,
            allowance_store,
            approval_manager,
            system_prompt: system_prompt.into(),
            created_at: Utc::now(),
            token_count: 0,
            metadata: SessionMetadata::default(),
            escape_handler: EscapeHandler::new(),
            budget_tracker,
            workspace_budget_tracker: None,
            workspace_path: None,
            model: None,
            is_subagent: true,
            plugin_context: None,
        }
    }

    /// Set the workspace path for this session.
    #[must_use]
    pub fn with_workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_path = Some(path.into());
        self
    }

    /// Set the model name for this session.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Replace the capability store with a persistent one.
    ///
    /// Call this after session construction when a persistent store is available
    /// (e.g. at daemon startup).
    #[must_use]
    pub fn with_capability_store(mut self, store: Arc<CapabilityStore>) -> Self {
        self.capabilities = store;
        self
    }

    /// Set the workspace cumulative budget tracker.
    #[must_use]
    pub fn with_workspace_budget(mut self, tracker: Arc<WorkspaceBudgetTracker>) -> Self {
        self.workspace_budget_tracker = Some(tracker);
        self
    }

    /// Import workspace-scoped allowances into this session.
    ///
    /// These allowances were previously persisted in the workspace `state.db`
    /// and are loaded when a session is created or resumed in the same workspace.
    pub fn import_workspace_allowances(
        &self,
        allowances: Vec<astrid_approval::allowance::Allowance>,
    ) {
        self.allowance_store.import_allowances(allowances);
    }

    /// Export workspace-scoped allowances from this session for persistence.
    #[must_use]
    pub fn export_workspace_allowances(&self) -> Vec<astrid_approval::allowance::Allowance> {
        self.allowance_store.export_workspace_allowances()
    }

    /// Replace the deferred resolution queue with a persistent one.
    ///
    /// This reconstructs the `ApprovalManager` with the new persistent queue.
    /// Call this after session construction when a persistent store is available.
    ///
    /// # Errors
    ///
    /// Returns an error if the persistent store cannot be initialized.
    pub async fn with_persistent_deferred_queue(
        mut self,
        store: astrid_storage::ScopedKvStore,
    ) -> Result<Self, crate::error::RuntimeError> {
        let deferred_queue = Arc::new(
            DeferredResolutionStore::with_persistence(store)
                .await
                .map_err(|e| crate::error::RuntimeError::StorageError(e.to_string()))?,
        );
        self.approval_manager = Arc::new(ApprovalManager::new(
            Arc::clone(&self.allowance_store),
            deferred_queue,
        ));
        Ok(self)
    }

    /// Add a message to the session.
    pub fn add_message(&mut self, message: Message) {
        // Rough token estimate (4 chars per token). This is a heuristic for
        // context-limit warnings, not billing. Real context windows top out at
        // ~200K tokens, so overflow of usize is not a practical concern.
        let msg_tokens = match &message.content {
            astrid_llm::MessageContent::Text(t) => t.len() / 4,
            _ => 100, // Rough estimate for tool calls
        };
        self.token_count = self.token_count.saturating_add(msg_tokens);
        self.messages.push(message);
    }

    /// Get the last N messages.
    #[must_use]
    pub fn last_messages(&self, n: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(n);
        &self.messages[start..]
    }

    /// Clear messages (keeping system prompt).
    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.token_count = 0;
    }

    /// Get session duration.
    #[must_use]
    pub fn duration(&self) -> chrono::Duration {
        // Safety: current time is always >= created_at
        #[allow(clippy::arithmetic_side_effects)]
        {
            Utc::now() - self.created_at
        }
    }

    /// Clean up session-scoped state.
    ///
    /// Clears session-only allowances, leaving workspace and persistent ones intact.
    pub fn end_session(&self) {
        self.allowance_store.clear_session_allowances();
    }

    /// Check if session is near context limit.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn is_near_limit(&self, max_tokens: usize, threshold: f32) -> bool {
        self.token_count as f32 > max_tokens as f32 * threshold
    }
}

/// Session metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Session title (generated or user-provided).
    pub title: Option<String>,
    /// Tags for organization.
    pub tags: Vec<String>,
    /// Number of turns.
    pub turn_count: usize,
    /// Number of tool calls.
    pub tool_call_count: usize,
    /// Number of approvals granted.
    pub approval_count: usize,
    /// Custom key-value metadata.
    pub custom: std::collections::HashMap<String, String>,
}

/// Serializable session state (for persistence).
///
/// Includes full security state: allowances, budget snapshot, escape handler
/// state, and workspace path. This ensures "Allow Session" approvals,
/// budget spend, and escape decisions survive daemon restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableSession {
    /// Session ID.
    pub id: String,
    /// User ID (hex).
    pub user_id: String,
    /// Messages.
    pub messages: Vec<SerializableMessage>,
    /// System prompt.
    pub system_prompt: String,
    /// Created at.
    pub created_at: DateTime<Utc>,
    /// Token count.
    pub token_count: usize,
    /// Metadata.
    pub metadata: SessionMetadata,
    /// Session allowances (persisted so "Allow Session" survives restart).
    #[serde(default)]
    pub allowances: Vec<Allowance>,
    /// Budget snapshot (persisted so budget is not reset on restart).
    #[serde(default)]
    pub budget_snapshot: Option<BudgetSnapshot>,
    /// Escape handler state (persisted so "Allow Always" paths survive).
    #[serde(default)]
    pub escape_state: Option<EscapeState>,
    /// Workspace path this session belongs to.
    #[serde(default)]
    pub workspace_path: Option<String>,
    /// Model used for this session (e.g. "claude-sonnet-4-20250514").
    #[serde(default)]
    pub model: Option<String>,
    /// Git state placeholder (branch, commit hash) for future worktree support.
    #[serde(default)]
    pub git_state: Option<GitState>,
}

/// Git repository state snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitState {
    /// Current branch name.
    pub branch: Option<String>,
    /// Current commit hash.
    pub commit: Option<String>,
}

impl GitState {
    /// Capture the current git state for a workspace path.
    ///
    /// Returns `None` if the path is not in a git repository or git is not available.
    #[must_use]
    pub fn capture(workspace_path: &std::path::Path) -> Option<Self> {
        let branch = std::process::Command::new("git")
            .args([
                "-C",
                &workspace_path.display().to_string(),
                "rev-parse",
                "--abbrev-ref",
                "HEAD",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());

        let commit = std::process::Command::new("git")
            .args([
                "-C",
                &workspace_path.display().to_string(),
                "rev-parse",
                "HEAD",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());

        // Only return Some if at least one field was captured.
        if branch.is_some() || commit.is_some() {
            Some(Self { branch, commit })
        } else {
            None
        }
    }
}

/// Serializable message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableMessage {
    /// Role.
    pub role: String,
    /// Content (JSON).
    pub content: serde_json::Value,
}

impl From<&AgentSession> for SerializableSession {
    fn from(session: &AgentSession) -> Self {
        Self {
            id: session.id.0.to_string(),
            user_id: hex::encode(session.user_id),
            messages: session
                .messages
                .iter()
                .map(|m| SerializableMessage {
                    role: match m.role {
                        astrid_llm::MessageRole::System => "system".to_string(),
                        astrid_llm::MessageRole::User => "user".to_string(),
                        astrid_llm::MessageRole::Assistant => "assistant".to_string(),
                        astrid_llm::MessageRole::Tool => "tool".to_string(),
                    },
                    content: serde_json::to_value(&m.content).unwrap_or_default(),
                })
                .collect(),
            system_prompt: session.system_prompt.clone(),
            created_at: session.created_at,
            token_count: session.token_count,
            metadata: session.metadata.clone(),
            allowances: session.allowance_store.export_session_allowances(),
            budget_snapshot: Some(session.budget_tracker.snapshot()),
            escape_state: Some(session.escape_handler.export_state()),
            workspace_path: session
                .workspace_path
                .as_ref()
                .map(|p| p.display().to_string()),
            model: session.model.clone(),
            git_state: session
                .workspace_path
                .as_ref()
                .and_then(|p| GitState::capture(p)),
        }
    }
}

impl SerializableSession {
    /// Convert back to an `AgentSession`.
    ///
    /// Restores full security state: allowances, budget, and escape handler.
    #[must_use]
    pub fn to_session(&self) -> AgentSession {
        let mut user_id = [0u8; 8];
        if let Ok(bytes) = hex::decode(&self.user_id)
            && bytes.len() >= 8
        {
            user_id.copy_from_slice(&bytes[..8]);
        }

        let id =
            uuid::Uuid::parse_str(&self.id).map_or_else(|_| SessionId::new(), SessionId::from_uuid);

        let messages: Vec<Message> = self
            .messages
            .iter()
            .filter_map(|m| {
                let content: astrid_llm::MessageContent =
                    serde_json::from_value(m.content.clone()).ok()?;
                let role = match m.role.as_str() {
                    "system" => astrid_llm::MessageRole::System,
                    "user" => astrid_llm::MessageRole::User,
                    "assistant" => astrid_llm::MessageRole::Assistant,
                    "tool" => astrid_llm::MessageRole::Tool,
                    _ => return None,
                };
                Some(Message { role, content })
            })
            .collect();

        let mut session = AgentSession::with_id(id, user_id, &self.system_prompt);
        session.messages = messages;
        session.created_at = self.created_at;
        session.token_count = self.token_count;
        session.metadata = self.metadata.clone();
        session.workspace_path = self.workspace_path.as_ref().map(PathBuf::from);
        session.model.clone_from(&self.model);

        // Restore session allowances
        if !self.allowances.is_empty() {
            session
                .allowance_store
                .import_allowances(self.allowances.clone());
        }

        // Restore budget from snapshot (prevents budget bypass via restart)
        if let Some(snapshot) = &self.budget_snapshot {
            session.budget_tracker = Arc::new(BudgetTracker::restore(snapshot.clone()));
        }

        // Restore escape handler state (preserves "AllowAlways" paths)
        if let Some(escape_state) = &self.escape_state {
            session.escape_handler.restore_state(escape_state.clone());
        }

        session
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrid_llm::Message;

    #[test]
    fn test_session_creation() {
        let session = AgentSession::new([0u8; 8], "You are helpful");
        assert!(session.messages.is_empty());
        assert_eq!(session.system_prompt, "You are helpful");
    }

    #[test]
    fn test_add_message() {
        let mut session = AgentSession::new([0u8; 8], "");
        session.add_message(Message::user("Hello"));
        session.add_message(Message::assistant("Hi!"));

        assert_eq!(session.messages.len(), 2);
        assert!(session.token_count > 0);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut session = AgentSession::new([1u8; 8], "Test prompt");
        session.add_message(Message::user("Hello"));
        session.add_message(Message::assistant("World"));

        let serializable = SerializableSession::from(&session);
        let restored = serializable.to_session();

        assert_eq!(restored.system_prompt, session.system_prompt);
        assert_eq!(restored.messages.len(), session.messages.len());
    }

    #[test]
    fn test_budget_snapshot_roundtrip() {
        let session = AgentSession::new([1u8; 8], "Test");
        session.budget_tracker.record_cost(42.5);

        let serializable = SerializableSession::from(&session);
        let restored = serializable.to_session();

        assert!((restored.budget_tracker.spent() - 42.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_workspace_path_roundtrip() {
        let session = AgentSession::new([1u8; 8], "Test").with_workspace("/home/user/project");

        let serializable = SerializableSession::from(&session);
        let restored = serializable.to_session();

        assert_eq!(
            restored.workspace_path,
            Some(PathBuf::from("/home/user/project"))
        );
    }

    #[test]
    fn test_with_shared_stores() {
        let parent = AgentSession::new([1u8; 8], "Parent");

        // Record some spend on the parent
        parent.budget_tracker.record_cost(10.0);

        // Create child with shared stores
        let child = AgentSession::with_shared_stores(
            SessionId::new(),
            [1u8; 8],
            "Child",
            Arc::clone(&parent.allowance_store),
            Arc::clone(&parent.capabilities),
            Arc::clone(&parent.budget_tracker),
        );

        // Budget spend is visible from child
        assert!((child.budget_tracker.spent() - 10.0).abs() < f64::EPSILON);

        // Child spend is visible from parent (same Arc)
        child.budget_tracker.record_cost(5.0);
        assert!((parent.budget_tracker.spent() - 15.0).abs() < f64::EPSILON);

        // Stores are the same Arc
        assert!(Arc::ptr_eq(&parent.budget_tracker, &child.budget_tracker));
        assert!(Arc::ptr_eq(&parent.allowance_store, &child.allowance_store));
        assert!(Arc::ptr_eq(&parent.capabilities, &child.capabilities));

        // Messages are independent
        assert!(child.messages.is_empty());

        // ApprovalManager is a different instance (fresh handler registration)
        assert!(!Arc::ptr_eq(
            &parent.approval_manager,
            &child.approval_manager
        ));
    }

    #[test]
    fn test_backwards_compatible_deserialization() {
        // Old session format without new fields should still deserialize
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "user_id": "0101010101010101",
            "messages": [],
            "system_prompt": "Test",
            "created_at": "2024-01-01T00:00:00Z",
            "token_count": 0,
            "metadata": {
                "title": null,
                "tags": [],
                "turn_count": 0,
                "tool_call_count": 0,
                "approval_count": 0,
                "custom": {}
            }
        }"#;

        let serializable: SerializableSession = serde_json::from_str(json).unwrap();
        let session = serializable.to_session();
        assert_eq!(session.system_prompt, "Test");
        assert!(session.workspace_path.is_none());
        assert!((session.budget_tracker.spent() - 0.0_f64).abs() < f64::EPSILON);
    }
}
