//! Agent manager for the gateway.

use crate::config::AgentConfig;
use crate::error::{GatewayError, GatewayResult};
use astrid_core::AgentId;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Status of an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    /// Agent is starting up.
    Starting,
    /// Agent is ready to handle requests.
    Ready,
    /// Agent is processing a request.
    Busy,
    /// Agent is paused.
    Paused,
    /// Agent is shutting down.
    ShuttingDown,
    /// Agent has stopped.
    Stopped,
    /// Agent encountered an error.
    Error,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Ready => write!(f, "ready"),
            Self::Busy => write!(f, "busy"),
            Self::Paused => write!(f, "paused"),
            Self::ShuttingDown => write!(f, "shutting_down"),
            Self::Stopped => write!(f, "stopped"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Handle to a running agent.
#[derive(Debug)]
pub struct AgentHandle {
    /// Agent ID.
    pub id: AgentId,

    /// Agent name.
    pub name: String,

    /// Current status.
    status: Arc<RwLock<AgentStatus>>,

    /// Configuration used to create this agent.
    pub config: AgentConfig,

    /// When the agent started.
    pub started_at: DateTime<Utc>,

    /// Last activity timestamp.
    last_activity: Arc<RwLock<DateTime<Utc>>>,

    /// Number of requests processed.
    request_count: Arc<RwLock<u64>>,

    /// Last error message (if any).
    last_error: Arc<RwLock<Option<String>>>,
}

impl AgentHandle {
    /// Create a new agent handle.
    #[must_use]
    pub fn new(id: AgentId, config: AgentConfig) -> Self {
        let now = Utc::now();
        Self {
            id,
            name: config.name.clone(),
            status: Arc::new(RwLock::new(AgentStatus::Starting)),
            config,
            started_at: now,
            last_activity: Arc::new(RwLock::new(now)),
            request_count: Arc::new(RwLock::new(0)),
            last_error: Arc::new(RwLock::new(None)),
        }
    }

    /// Get current status.
    pub async fn status(&self) -> AgentStatus {
        *self.status.read().await
    }

    /// Set status.
    pub async fn set_status(&self, status: AgentStatus) {
        *self.status.write().await = status;
    }

    /// Get last activity time.
    pub async fn last_activity(&self) -> DateTime<Utc> {
        *self.last_activity.read().await
    }

    /// Update last activity time.
    pub async fn touch(&self) {
        *self.last_activity.write().await = Utc::now();
    }

    /// Increment request count.
    pub async fn increment_requests(&self) {
        let mut count = self.request_count.write().await;
        *count = count.saturating_add(1);
    }

    /// Get request count.
    pub async fn request_count(&self) -> u64 {
        *self.request_count.read().await
    }

    /// Set error.
    pub async fn set_error(&self, error: impl Into<String>) {
        *self.last_error.write().await = Some(error.into());
        self.set_status(AgentStatus::Error).await;
    }

    /// Get last error.
    pub async fn last_error(&self) -> Option<String> {
        self.last_error.read().await.clone()
    }

    /// Clear error and set status back to ready.
    pub async fn clear_error(&self) {
        *self.last_error.write().await = None;
        self.set_status(AgentStatus::Ready).await;
    }

    /// Check if agent is available for requests.
    pub async fn is_available(&self) -> bool {
        matches!(self.status().await, AgentStatus::Ready)
    }
}

/// Manager for multiple agent instances.
#[derive(Debug, Default)]
pub struct AgentManager {
    /// Running agents by ID.
    agents: HashMap<AgentId, Arc<AgentHandle>>,

    /// Name to ID mapping.
    name_to_id: HashMap<String, AgentId>,
}

impl AgentManager {
    /// Create a new agent manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new agent with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if an agent with the same name already exists.
    pub async fn start(&mut self, config: AgentConfig) -> GatewayResult<AgentId> {
        if self.name_to_id.contains_key(&config.name) {
            return Err(GatewayError::Agent(format!(
                "agent with name '{}' already exists",
                config.name
            )));
        }

        let id = AgentId::new();
        let handle = Arc::new(AgentHandle::new(id.clone(), config.clone()));

        // Mark as ready (in real implementation, would initialize resources)
        handle.set_status(AgentStatus::Ready).await;

        self.agents.insert(id.clone(), handle);
        self.name_to_id.insert(config.name, id.clone());

        Ok(id)
    }

    /// Stop an agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent is not found.
    pub async fn stop(&mut self, id: &AgentId) -> GatewayResult<()> {
        let handle = self
            .agents
            .get(id)
            .ok_or_else(|| GatewayError::Agent(format!("agent not found: {id}")))?;

        handle.set_status(AgentStatus::ShuttingDown).await;

        // In real implementation, would gracefully shutdown
        handle.set_status(AgentStatus::Stopped).await;

        // Remove from maps
        self.name_to_id.retain(|_, v| v != id);
        self.agents.remove(id);

        Ok(())
    }

    /// Stop an agent by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent is not found.
    pub async fn stop_by_name(&mut self, name: &str) -> GatewayResult<()> {
        let id = self
            .name_to_id
            .get(name)
            .cloned()
            .ok_or_else(|| GatewayError::Agent(format!("agent not found: {name}")))?;

        self.stop(&id).await
    }

    /// Get an agent by ID.
    #[must_use]
    pub fn get(&self, id: &AgentId) -> Option<Arc<AgentHandle>> {
        self.agents.get(id).cloned()
    }

    /// Get an agent by name.
    #[must_use]
    pub fn get_by_name(&self, name: &str) -> Option<Arc<AgentHandle>> {
        self.name_to_id
            .get(name)
            .and_then(|id| self.agents.get(id))
            .cloned()
    }

    /// List all agents.
    #[must_use]
    pub fn list(&self) -> Vec<Arc<AgentHandle>> {
        self.agents.values().cloned().collect()
    }

    /// List agent names.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.name_to_id.keys().map(String::as_str).collect()
    }

    /// Get count of running agents.
    #[must_use]
    pub fn count(&self) -> usize {
        self.agents.len()
    }

    /// Check if an agent with the given name exists.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.name_to_id.contains_key(name)
    }

    /// Pause an agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent is not found.
    pub async fn pause(&self, id: &AgentId) -> GatewayResult<()> {
        let handle = self
            .agents
            .get(id)
            .ok_or_else(|| GatewayError::Agent(format!("agent not found: {id}")))?;

        handle.set_status(AgentStatus::Paused).await;
        Ok(())
    }

    /// Resume a paused agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent is not found or not paused.
    pub async fn resume(&self, id: &AgentId) -> GatewayResult<()> {
        let handle = self
            .agents
            .get(id)
            .ok_or_else(|| GatewayError::Agent(format!("agent not found: {id}")))?;

        if handle.status().await != AgentStatus::Paused {
            return Err(GatewayError::Agent("agent is not paused".into()));
        }

        handle.set_status(AgentStatus::Ready).await;
        Ok(())
    }

    /// Stop all agents.
    pub async fn stop_all(&mut self) {
        let ids: Vec<_> = self.agents.keys().cloned().collect();
        for id in ids {
            let _ = self.stop(&id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent_config(name: &str) -> AgentConfig {
        AgentConfig {
            name: name.into(),
            description: None,
            model: None,
            system_prompt: None,
            max_context_tokens: None,
            subagents: None,
            timeouts: None,
            channels: vec![],
            auto_start: false,
        }
    }

    #[tokio::test]
    async fn test_agent_handle() {
        let config = test_agent_config("test");
        let handle = AgentHandle::new(AgentId::new(), config);

        assert_eq!(handle.status().await, AgentStatus::Starting);

        handle.set_status(AgentStatus::Ready).await;
        assert!(handle.is_available().await);

        handle.touch().await;
        handle.increment_requests().await;
        assert_eq!(handle.request_count().await, 1);
    }

    #[tokio::test]
    async fn test_agent_error() {
        let config = test_agent_config("test");
        let handle = AgentHandle::new(AgentId::new(), config);

        handle.set_error("something went wrong").await;
        assert_eq!(handle.status().await, AgentStatus::Error);
        assert_eq!(
            handle.last_error().await,
            Some("something went wrong".into())
        );

        handle.clear_error().await;
        assert_eq!(handle.status().await, AgentStatus::Ready);
        assert!(handle.last_error().await.is_none());
    }

    #[tokio::test]
    async fn test_manager_start_stop() {
        let mut manager = AgentManager::new();

        let config = test_agent_config("test-agent");
        let id = manager.start(config).await.unwrap();

        assert!(manager.has("test-agent"));
        assert_eq!(manager.count(), 1);

        let handle = manager.get(&id).unwrap();
        assert_eq!(handle.status().await, AgentStatus::Ready);

        manager.stop(&id).await.unwrap();
        assert!(!manager.has("test-agent"));
        assert_eq!(manager.count(), 0);
    }

    #[tokio::test]
    async fn test_manager_duplicate_name() {
        let mut manager = AgentManager::new();

        let config = test_agent_config("agent");
        manager.start(config.clone()).await.unwrap();

        let result = manager.start(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_manager_get_by_name() {
        let mut manager = AgentManager::new();

        let config = test_agent_config("my-agent");
        let id = manager.start(config).await.unwrap();

        let handle = manager.get_by_name("my-agent").unwrap();
        assert_eq!(handle.id, id);
    }

    #[tokio::test]
    async fn test_manager_pause_resume() {
        let mut manager = AgentManager::new();

        let config = test_agent_config("agent");
        let id = manager.start(config).await.unwrap();

        manager.pause(&id).await.unwrap();
        assert_eq!(
            manager.get(&id).unwrap().status().await,
            AgentStatus::Paused
        );

        manager.resume(&id).await.unwrap();
        assert_eq!(manager.get(&id).unwrap().status().await, AgentStatus::Ready);
    }

    #[tokio::test]
    async fn test_manager_stop_all() {
        let mut manager = AgentManager::new();

        manager.start(test_agent_config("agent1")).await.unwrap();
        manager.start(test_agent_config("agent2")).await.unwrap();
        assert_eq!(manager.count(), 2);

        manager.stop_all().await;
        assert_eq!(manager.count(), 0);
    }
}
