//! State persistence for the gateway.

use crate::error::{GatewayError, GatewayResult};
use astralis_core::{Version, Versioned};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Persisted gateway state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    /// When this state was last saved.
    pub saved_at: Option<DateTime<Utc>>,

    /// Active agent states.
    pub agents: HashMap<String, AgentState>,

    /// Pending approvals.
    pub pending_approvals: Vec<PendingApproval>,

    /// Queued tasks.
    pub queued_tasks: Vec<QueuedTask>,

    /// Subagent states.
    pub subagents: HashMap<String, SubAgentState>,
}

/// State of an individual agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentState {
    /// Agent name.
    pub name: String,

    /// Current session ID.
    pub session_id: Option<String>,

    /// Last activity time.
    pub last_activity: Option<DateTime<Utc>>,

    /// Request count.
    pub request_count: u64,

    /// Error count.
    pub error_count: u64,

    /// Custom metadata.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// State of a subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentState {
    /// Subagent ID.
    pub id: String,

    /// Parent subagent ID.
    pub parent_id: Option<String>,

    /// Task description.
    pub task: String,

    /// Current depth.
    pub depth: usize,

    /// Status.
    pub status: String,

    /// Started at.
    pub started_at: DateTime<Utc>,

    /// Completed at.
    pub completed_at: Option<DateTime<Utc>>,
}

/// A pending approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique ID for this approval.
    pub id: String,

    /// Agent that requested approval.
    pub agent_name: String,

    /// Session ID.
    pub session_id: String,

    /// Type of approval.
    pub approval_type: String,

    /// Description of what's being approved.
    pub description: String,

    /// When the approval was requested.
    pub requested_at: DateTime<Utc>,

    /// When the approval expires.
    pub expires_at: Option<DateTime<Utc>>,

    /// Risk level.
    pub risk_level: String,

    /// Tool being called (if applicable).
    pub tool_name: Option<String>,

    /// Additional context.
    #[serde(default)]
    pub context: HashMap<String, serde_json::Value>,
}

/// A queued task waiting for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedTask {
    /// Unique ID for this task.
    pub id: String,

    /// Agent to execute on.
    pub agent_name: String,

    /// Task type.
    pub task_type: String,

    /// Task payload.
    pub payload: serde_json::Value,

    /// When the task was queued.
    pub queued_at: DateTime<Utc>,

    /// Priority (higher = more urgent).
    pub priority: i32,

    /// Number of retry attempts.
    pub retry_count: u32,

    /// Last error (if retried).
    pub last_error: Option<String>,
}

impl PersistedState {
    /// Current state format version.
    pub const VERSION: Version = Version::new(1, 0, 0);

    /// Create a new empty state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            saved_at: None,
            agents: HashMap::new(),
            pending_approvals: Vec::new(),
            queued_tasks: Vec::new(),
            subagents: HashMap::new(),
        }
    }

    /// Load state from a file, using `Versioned<T>` for safe migration.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, parsed, or the version is too new.
    pub fn load<P: AsRef<Path>>(path: P) -> GatewayResult<Self> {
        let contents = std::fs::read_to_string(path.as_ref())?;

        let versioned = serde_json::from_str::<Versioned<Self>>(&contents)?;
        if versioned.version.is_newer_than(&Self::VERSION) {
            return Err(GatewayError::State(format!(
                "state version {} is newer than supported version {}",
                versioned.version,
                Self::VERSION
            )));
        }
        Ok(versioned.into_inner())
    }

    /// Load state from a file, returning default if not found.
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Save state to a file wrapped in `Versioned<T>`.
    ///
    /// On Unix systems, the file is created with restrictive permissions (0600)
    /// to protect sensitive state data.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save<P: AsRef<Path>>(&mut self, path: P) -> GatewayResult<()> {
        self.saved_at = Some(Utc::now());

        // Ensure parent directory exists
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let versioned = Versioned::with_version(Self::VERSION, &self);
        let contents = serde_json::to_string_pretty(&versioned)?;
        std::fs::write(path.as_ref(), &contents)?;

        // Set restrictive permissions on Unix (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path.as_ref(), permissions)?;
        }

        Ok(())
    }

    /// Create a checkpoint (saves with timestamp suffix).
    ///
    /// # Errors
    ///
    /// Returns an error if the checkpoint cannot be created.
    pub fn checkpoint<P: AsRef<Path>>(
        &mut self,
        base_path: P,
    ) -> GatewayResult<std::path::PathBuf> {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let path = base_path.as_ref();
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let ext = path.extension().unwrap_or_default().to_string_lossy();

        let checkpoint_path = path.with_file_name(format!("{stem}_{timestamp}.{ext}"));
        self.save(&checkpoint_path)?;
        Ok(checkpoint_path)
    }

    /// Get agent state.
    #[must_use]
    pub fn agent(&self, name: &str) -> Option<&AgentState> {
        self.agents.get(name)
    }

    /// Get mutable agent state.
    pub fn agent_mut(&mut self, name: &str) -> Option<&mut AgentState> {
        self.agents.get_mut(name)
    }

    /// Set agent state.
    pub fn set_agent(&mut self, name: impl Into<String>, state: AgentState) {
        self.agents.insert(name.into(), state);
    }

    /// Remove agent state.
    pub fn remove_agent(&mut self, name: &str) -> Option<AgentState> {
        self.agents.remove(name)
    }

    /// Add a pending approval.
    pub fn add_pending_approval(&mut self, approval: PendingApproval) {
        self.pending_approvals.push(approval);
    }

    /// Remove a pending approval by ID.
    pub fn remove_pending_approval(&mut self, id: &str) -> Option<PendingApproval> {
        if let Some(idx) = self.pending_approvals.iter().position(|a| a.id == id) {
            Some(self.pending_approvals.remove(idx))
        } else {
            None
        }
    }

    /// Get pending approvals for an agent.
    #[must_use]
    pub fn agent_pending_approvals(&self, agent_name: &str) -> Vec<&PendingApproval> {
        self.pending_approvals
            .iter()
            .filter(|a| a.agent_name == agent_name)
            .collect()
    }

    /// Remove expired approvals.
    pub fn prune_expired_approvals(&mut self) {
        let now = Utc::now();
        self.pending_approvals
            .retain(|a| a.expires_at.is_none_or(|exp| exp > now));
    }

    /// Add a queued task.
    pub fn queue_task(&mut self, task: QueuedTask) {
        self.queued_tasks.push(task);
        // Sort by priority (highest first)
        self.queued_tasks
            .sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Pop the highest priority task for an agent.
    pub fn pop_task(&mut self, agent_name: &str) -> Option<QueuedTask> {
        if let Some(idx) = self
            .queued_tasks
            .iter()
            .position(|t| t.agent_name == agent_name)
        {
            Some(self.queued_tasks.remove(idx))
        } else {
            None
        }
    }

    /// Get queued task count.
    #[must_use]
    pub fn queued_task_count(&self) -> usize {
        self.queued_tasks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_state_new() {
        let state = PersistedState::new();
        assert!(state.agents.is_empty());
        assert!(state.pending_approvals.is_empty());
        assert!(state.queued_tasks.is_empty());
    }

    #[test]
    fn test_state_save_load() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.json");

        let mut state = PersistedState::new();
        state.set_agent(
            "test-agent",
            AgentState {
                name: "test-agent".into(),
                session_id: Some("session-1".into()),
                last_activity: Some(Utc::now()),
                request_count: 10,
                error_count: 1,
                metadata: HashMap::new(),
            },
        );

        state.save(&path).unwrap();
        assert!(state.saved_at.is_some());

        let loaded = PersistedState::load(&path).unwrap();
        assert!(loaded.agent("test-agent").is_some());
        assert_eq!(loaded.agent("test-agent").unwrap().request_count, 10);
    }

    #[test]
    fn test_state_versioned_format() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.json");

        let mut state = PersistedState::new();
        state.save(&path).unwrap();

        // Verify the on-disk format contains version info
        let contents = std::fs::read_to_string(&path).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(raw.get("version").is_some());
        assert!(raw.get("data").is_some());
    }

    #[test]
    fn test_state_checkpoint() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.json");

        let mut state = PersistedState::new();
        let checkpoint_path = state.checkpoint(&path).unwrap();

        assert!(checkpoint_path.exists());
        assert!(checkpoint_path.to_string_lossy().contains("state_"));
    }

    #[test]
    fn test_pending_approvals() {
        let mut state = PersistedState::new();

        state.add_pending_approval(PendingApproval {
            id: "approval-1".into(),
            agent_name: "agent-1".into(),
            session_id: "session-1".into(),
            approval_type: "tool_call".into(),
            description: "Run command".into(),
            requested_at: Utc::now(),
            expires_at: None,
            risk_level: "high".into(),
            tool_name: Some("execute".into()),
            context: HashMap::new(),
        });

        assert_eq!(state.pending_approvals.len(), 1);

        let approvals = state.agent_pending_approvals("agent-1");
        assert_eq!(approvals.len(), 1);

        let removed = state.remove_pending_approval("approval-1");
        assert!(removed.is_some());
        assert!(state.pending_approvals.is_empty());
    }

    #[test]
    fn test_queued_tasks() {
        let mut state = PersistedState::new();

        state.queue_task(QueuedTask {
            id: "task-1".into(),
            agent_name: "agent-1".into(),
            task_type: "message".into(),
            payload: serde_json::json!({"text": "hello"}),
            queued_at: Utc::now(),
            priority: 1,
            retry_count: 0,
            last_error: None,
        });

        state.queue_task(QueuedTask {
            id: "task-2".into(),
            agent_name: "agent-1".into(),
            task_type: "message".into(),
            payload: serde_json::json!({"text": "urgent"}),
            queued_at: Utc::now(),
            priority: 10, // Higher priority
            retry_count: 0,
            last_error: None,
        });

        assert_eq!(state.queued_task_count(), 2);

        // Should get higher priority task first
        let task = state.pop_task("agent-1").unwrap();
        assert_eq!(task.id, "task-2");

        let task = state.pop_task("agent-1").unwrap();
        assert_eq!(task.id, "task-1");
    }
}
