//! MCP task management (Nov 2025 spec).
//!
//! Tasks represent long-running operations that can be tracked,
//! monitored, and cancelled.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// State of an MCP task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Task is waiting to start.
    Pending,
    /// Task is currently running.
    Running,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Task was cancelled.
    Cancelled,
}

impl TaskState {
    /// Check if the task is in a terminal state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Check if the task is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// An MCP task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique task identifier.
    pub id: Uuid,
    /// Server that owns this task.
    pub server: String,
    /// Task name/type.
    pub name: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Current state.
    pub state: TaskState,
    /// Progress (0.0 - 1.0).
    pub progress: Option<f32>,
    /// Progress message.
    pub progress_message: Option<String>,
    /// When the task was created.
    pub created_at: DateTime<Utc>,
    /// When the task started running.
    pub started_at: Option<DateTime<Utc>>,
    /// When the task finished.
    pub finished_at: Option<DateTime<Utc>>,
    /// Result if completed.
    pub result: Option<serde_json::Value>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Task {
    /// Create a new pending task.
    #[must_use]
    pub fn new(server: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            server: server.into(),
            name: name.into(),
            description: None,
            state: TaskState::Pending,
            progress: None,
            progress_message: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            result: None,
            error: None,
            metadata: HashMap::new(),
        }
    }

    /// Set the description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add metadata.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Mark the task as running.
    pub fn start(&mut self) {
        self.state = TaskState::Running;
        self.started_at = Some(Utc::now());
    }

    /// Update progress.
    pub fn update_progress(&mut self, progress: f32, message: Option<String>) {
        self.progress = Some(progress.clamp(0.0, 1.0));
        self.progress_message = message;
    }

    /// Mark the task as completed.
    pub fn complete(&mut self, result: Option<serde_json::Value>) {
        self.state = TaskState::Completed;
        self.finished_at = Some(Utc::now());
        self.progress = Some(1.0);
        self.result = result;
    }

    /// Mark the task as failed.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.state = TaskState::Failed;
        self.finished_at = Some(Utc::now());
        self.error = Some(error.into());
    }

    /// Mark the task as cancelled.
    pub fn cancel(&mut self) {
        self.state = TaskState::Cancelled;
        self.finished_at = Some(Utc::now());
    }

    /// Get the duration of the task.
    #[must_use]
    #[allow(clippy::arithmetic_side_effects)] // end >= start and now >= start by construction
    pub fn duration(&self) -> Option<chrono::Duration> {
        match (self.started_at, self.finished_at) {
            (Some(start), Some(end)) => Some(end - start),
            (Some(start), None) if self.state == TaskState::Running => Some(Utc::now() - start),
            _ => None,
        }
    }

    /// Get the duration in milliseconds.
    #[must_use]
    pub fn duration_ms(&self) -> Option<i64> {
        self.duration().map(|d| d.num_milliseconds())
    }
}

/// Manager for MCP tasks.
#[derive(Debug)]
pub struct TaskManager {
    /// All tasks by ID.
    tasks: Arc<RwLock<HashMap<Uuid, Task>>>,
    /// Tasks by server.
    by_server: Arc<RwLock<HashMap<String, Vec<Uuid>>>>,
    /// Maximum number of tasks to keep per server.
    max_tasks_per_server: usize,
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskManager {
    /// Create a new task manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            by_server: Arc::new(RwLock::new(HashMap::new())),
            max_tasks_per_server: 100,
        }
    }

    /// Create a new task manager with a custom limit.
    #[must_use]
    pub fn with_limit(max_tasks_per_server: usize) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            by_server: Arc::new(RwLock::new(HashMap::new())),
            max_tasks_per_server,
        }
    }

    /// Create a new task.
    pub async fn create_task(&self, server: impl Into<String>, name: impl Into<String>) -> Task {
        let task = Task::new(server, name);
        let task_id = task.id;
        let server_name = task.server.clone();

        // Store the task
        {
            let mut tasks = self.tasks.write().await;
            tasks.insert(task_id, task.clone());
        }

        // Update server index
        {
            let mut by_server = self.by_server.write().await;
            let server_tasks = by_server.entry(server_name.clone()).or_default();
            server_tasks.push(task_id);

            // Prune old tasks if over limit
            if server_tasks.len() > self.max_tasks_per_server {
                // Safety: checked `len() > max_tasks_per_server` above
                #[allow(clippy::arithmetic_side_effects)]
                let drain_count = server_tasks.len() - self.max_tasks_per_server;
                let to_remove: Vec<_> = server_tasks.drain(..drain_count).collect();

                let mut tasks = self.tasks.write().await;
                for id in to_remove {
                    tasks.remove(&id);
                }
            }
        }

        task
    }

    /// Get a task by ID.
    pub async fn get_task(&self, task_id: Uuid) -> Option<Task> {
        let tasks = self.tasks.read().await;
        tasks.get(&task_id).cloned()
    }

    /// List all tasks for a server.
    pub async fn list_tasks(&self, server: &str) -> Vec<Task> {
        let by_server = self.by_server.read().await;
        let tasks = self.tasks.read().await;

        by_server
            .get(server)
            .map(|ids| ids.iter().filter_map(|id| tasks.get(id).cloned()).collect())
            .unwrap_or_default()
    }

    /// List running tasks for a server.
    pub async fn list_running_tasks(&self, server: &str) -> Vec<Task> {
        self.list_tasks(server)
            .await
            .into_iter()
            .filter(|t| t.state.is_running())
            .collect()
    }

    /// Update a task.
    ///
    /// Returns `true` if the task was found and updated.
    pub async fn update_task<F>(&self, task_id: Uuid, updater: F) -> bool
    where
        F: FnOnce(&mut Task),
    {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(&task_id) {
            updater(task);
            true
        } else {
            false
        }
    }

    /// Cancel a task.
    ///
    /// Returns `true` if the task was found and cancelled.
    pub async fn cancel_task(&self, task_id: Uuid) -> bool {
        self.update_task(task_id, |task| {
            if !task.state.is_terminal() {
                task.cancel();
            }
        })
        .await
    }

    /// Remove a task.
    ///
    /// Returns the removed task if found.
    pub async fn remove_task(&self, task_id: Uuid) -> Option<Task> {
        let task = {
            let mut tasks = self.tasks.write().await;
            tasks.remove(&task_id)
        };

        if let Some(ref t) = task {
            let mut by_server = self.by_server.write().await;
            if let Some(server_tasks) = by_server.get_mut(&t.server) {
                server_tasks.retain(|id| *id != task_id);
            }
        }

        task
    }

    /// Clean up completed tasks older than the given duration.
    pub async fn cleanup_old_tasks(&self, max_age: chrono::Duration) {
        // Safety: subtracting a positive duration from current time
        #[allow(clippy::arithmetic_side_effects)]
        let cutoff = Utc::now() - max_age;

        let to_remove: Vec<Uuid> = {
            let tasks = self.tasks.read().await;
            tasks
                .values()
                .filter(|t| t.state.is_terminal() && t.finished_at.is_some_and(|f| f < cutoff))
                .map(|t| t.id)
                .collect()
        };

        for id in to_remove {
            self.remove_task(id).await;
        }
    }

    /// Get total task count.
    pub async fn total_count(&self) -> usize {
        self.tasks.read().await.len()
    }

    /// Get running task count.
    pub async fn running_count(&self) -> usize {
        self.tasks
            .read()
            .await
            .values()
            .filter(|t| t.state.is_running())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_state() {
        assert!(!TaskState::Pending.is_terminal());
        assert!(!TaskState::Running.is_terminal());
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Cancelled.is_terminal());
    }

    #[test]
    fn test_task_lifecycle() {
        let mut task = Task::new("server", "test_task");
        assert_eq!(task.state, TaskState::Pending);

        task.start();
        assert_eq!(task.state, TaskState::Running);
        assert!(task.started_at.is_some());

        task.update_progress(0.5, Some("halfway".into()));
        assert_eq!(task.progress, Some(0.5));
        assert_eq!(task.progress_message, Some("halfway".to_string()));

        task.complete(Some(serde_json::json!({"result": "success"})));
        assert_eq!(task.state, TaskState::Completed);
        assert!(task.finished_at.is_some());
        assert_eq!(task.progress, Some(1.0));
    }

    #[test]
    fn test_task_failure() {
        let mut task = Task::new("server", "failing_task");
        task.start();
        task.fail("Something went wrong");

        assert_eq!(task.state, TaskState::Failed);
        assert_eq!(task.error, Some("Something went wrong".to_string()));
    }

    #[tokio::test]
    async fn test_task_manager_create() {
        let manager = TaskManager::new();
        let task = manager.create_task("server1", "task1").await;

        assert_eq!(task.server, "server1");
        assert_eq!(task.name, "task1");

        let retrieved = manager.get_task(task.id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, task.id);
    }

    #[tokio::test]
    async fn test_task_manager_list() {
        let manager = TaskManager::new();
        manager.create_task("server1", "task1").await;
        manager.create_task("server1", "task2").await;
        manager.create_task("server2", "task3").await;

        let server1_tasks = manager.list_tasks("server1").await;
        assert_eq!(server1_tasks.len(), 2);

        let server2_tasks = manager.list_tasks("server2").await;
        assert_eq!(server2_tasks.len(), 1);
    }

    #[tokio::test]
    async fn test_task_manager_update() {
        let manager = TaskManager::new();
        let task = manager.create_task("server", "task").await;

        let updated = manager
            .update_task(task.id, |t| {
                t.start();
            })
            .await;
        assert!(updated);

        let task = manager.get_task(task.id).await.unwrap();
        assert_eq!(task.state, TaskState::Running);
    }

    #[tokio::test]
    async fn test_task_manager_cancel() {
        let manager = TaskManager::new();
        let task = manager.create_task("server", "task").await;
        manager
            .update_task(task.id, |t| {
                t.start();
            })
            .await;

        let cancelled = manager.cancel_task(task.id).await;
        assert!(cancelled);

        let task = manager.get_task(task.id).await.unwrap();
        assert_eq!(task.state, TaskState::Cancelled);
    }

    #[tokio::test]
    async fn test_task_manager_limits() {
        let manager = TaskManager::with_limit(5);

        // Create more tasks than the limit
        for i in 0..10 {
            manager.create_task("server", format!("task{i}")).await;
        }

        let tasks = manager.list_tasks("server").await;
        assert_eq!(tasks.len(), 5);
    }
}
