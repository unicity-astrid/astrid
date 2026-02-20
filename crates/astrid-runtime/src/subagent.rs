//! Subagent pool management.
//!
//! Manages lifecycle (spawn, cancel, depth/concurrency enforcement) for sub-agents.

use crate::error::{RuntimeError, RuntimeResult};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Unique identifier for a subagent instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubAgentId(String);

impl SubAgentId {
    /// Create a new random subagent ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for SubAgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SubAgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Status of a subagent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubAgentStatus {
    /// Subagent is initializing.
    Initializing,
    /// Subagent is running.
    Running,
    /// Subagent completed successfully.
    Completed,
    /// Subagent failed.
    Failed,
    /// Subagent was cancelled.
    Cancelled,
    /// Subagent timed out.
    TimedOut,
}

impl SubAgentStatus {
    /// Whether this status is terminal (done).
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

impl std::fmt::Display for SubAgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initializing => write!(f, "initializing"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::TimedOut => write!(f, "timed_out"),
        }
    }
}

/// Handle to a running subagent.
#[derive(Debug)]
pub struct SubAgentHandle {
    /// Subagent ID.
    pub id: SubAgentId,

    /// Parent agent ID (if nested).
    pub parent_id: Option<SubAgentId>,

    /// Task description.
    pub task: String,

    /// Current depth (0 for first-level subagents).
    pub depth: usize,

    /// Current status (async access).
    status: Arc<RwLock<SubAgentStatus>>,

    /// Final status snapshot (sync access, set when entering terminal state).
    final_status: Arc<std::sync::Mutex<Option<SubAgentStatus>>>,

    /// When the subagent started.
    pub started_at: DateTime<Utc>,

    /// When the subagent completed (if done).
    completed_at: Arc<RwLock<Option<DateTime<Utc>>>>,

    /// Result (if completed).
    result: Arc<RwLock<Option<String>>>,

    /// Error message (if failed).
    error: Arc<RwLock<Option<String>>>,

    /// Semaphore permit — explicitly released when the handle leaves the active pool.
    permit: std::sync::Mutex<Option<OwnedSemaphorePermit>>,
}

impl SubAgentHandle {
    /// Create a new subagent handle.
    #[must_use]
    pub fn new(
        task: impl Into<String>,
        parent_id: Option<SubAgentId>,
        depth: usize,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            id: SubAgentId::new(),
            parent_id,
            task: task.into(),
            depth,
            status: Arc::new(RwLock::new(SubAgentStatus::Initializing)),
            final_status: Arc::new(std::sync::Mutex::new(None)),
            started_at: Utc::now(),
            completed_at: Arc::new(RwLock::new(None)),
            result: Arc::new(RwLock::new(None)),
            error: Arc::new(RwLock::new(None)),
            permit: std::sync::Mutex::new(permit),
        }
    }

    /// Get current status.
    pub async fn status(&self) -> SubAgentStatus {
        *self.status.read().await
    }

    /// Get final status synchronously (only set when terminal).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn final_status(&self) -> Option<SubAgentStatus> {
        *self
            .final_status
            .lock()
            .expect("final_status mutex poisoned")
    }

    /// Set status.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub async fn set_status(&self, status: SubAgentStatus) {
        *self.status.write().await = status;
        if status.is_terminal() {
            *self.completed_at.write().await = Some(Utc::now());
            *self
                .final_status
                .lock()
                .expect("final_status mutex poisoned") = Some(status);
        }
    }

    /// Mark as running.
    pub async fn mark_running(&self) {
        self.set_status(SubAgentStatus::Running).await;
    }

    /// Mark as completed with result.
    pub async fn complete(&self, result: impl Into<String>) {
        *self.result.write().await = Some(result.into());
        self.set_status(SubAgentStatus::Completed).await;
    }

    /// Mark as failed with error.
    pub async fn fail(&self, error: impl Into<String>) {
        *self.error.write().await = Some(error.into());
        self.set_status(SubAgentStatus::Failed).await;
    }

    /// Mark as cancelled.
    pub async fn cancel(&self) {
        self.set_status(SubAgentStatus::Cancelled).await;
    }

    /// Mark as timed out.
    pub async fn timeout(&self) {
        self.set_status(SubAgentStatus::TimedOut).await;
    }

    /// Get result (if completed).
    pub async fn result(&self) -> Option<String> {
        self.result.read().await.clone()
    }

    /// Get error (if failed).
    pub async fn error(&self) -> Option<String> {
        self.error.read().await.clone()
    }

    /// Get completion time (if done).
    pub async fn completed_at(&self) -> Option<DateTime<Utc>> {
        *self.completed_at.read().await
    }

    /// Get duration (if completed).
    #[allow(clippy::arithmetic_side_effects)] // completed_at is always >= started_at
    pub async fn duration(&self) -> Option<chrono::Duration> {
        self.completed_at()
            .await
            .map(|completed| completed - self.started_at)
    }

    /// Check if done (completed, failed, cancelled, or timed out).
    pub async fn is_done(&self) -> bool {
        self.status().await.is_terminal()
    }

    /// Release the semaphore permit (called when moving out of the active pool).
    fn release_permit(&self) {
        let _ = self.permit.lock().expect("permit mutex poisoned").take();
    }
}

/// Default maximum history size before FIFO eviction.
const DEFAULT_MAX_HISTORY: usize = 1000;

/// Pool for managing subagent instances.
#[derive(Debug)]
pub struct SubAgentPool {
    /// Maximum concurrent subagents.
    max_concurrent: usize,

    /// Maximum nesting depth.
    max_depth: usize,

    /// Maximum completed history entries before FIFO eviction.
    max_history: usize,

    /// Concurrency semaphore.
    semaphore: Arc<Semaphore>,

    /// Active subagents.
    active: Arc<RwLock<HashMap<SubAgentId, Arc<SubAgentHandle>>>>,

    /// Completed subagents (for history).
    completed: Arc<RwLock<Vec<Arc<SubAgentHandle>>>>,

    /// Notified when the active pool becomes empty.
    completion_notify: Arc<Notify>,

    /// Cooperative cancellation token for all sub-agents in this pool.
    cancellation_token: CancellationToken,
}

impl SubAgentPool {
    /// Create a new subagent pool with default history limit (1000 entries).
    #[must_use]
    pub fn new(max_concurrent: usize, max_depth: usize) -> Self {
        Self::with_max_history(max_concurrent, max_depth, DEFAULT_MAX_HISTORY)
    }

    /// Create a new subagent pool with explicit history limit.
    #[must_use]
    pub fn with_max_history(max_concurrent: usize, max_depth: usize, max_history: usize) -> Self {
        Self {
            max_concurrent,
            max_depth,
            max_history,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            active: Arc::new(RwLock::new(HashMap::new())),
            completed: Arc::new(RwLock::new(Vec::new())),
            completion_notify: Arc::new(Notify::new()),
            cancellation_token: CancellationToken::new(),
        }
    }

    /// Spawn a new subagent.
    ///
    /// # Errors
    ///
    /// Returns an error if the maximum subagent depth is exceeded or concurrency limit is reached.
    pub async fn spawn(
        &self,
        task: impl Into<String>,
        parent_id: Option<SubAgentId>,
    ) -> RuntimeResult<Arc<SubAgentHandle>> {
        let depth = if parent_id.is_some() {
            // Find parent depth
            let active = self.active.read().await;
            if let Some(parent) = parent_id.as_ref().and_then(|id| active.get(id)) {
                parent.depth.checked_add(1).ok_or_else(|| {
                    RuntimeError::SubAgentError("subagent depth overflow".to_string())
                })?
            } else {
                1
            }
        } else {
            0
        };

        if depth >= self.max_depth {
            return Err(RuntimeError::SubAgentError(format!(
                "maximum subagent depth ({}) exceeded",
                self.max_depth
            )));
        }

        // Try to acquire semaphore permit
        let permit = self.semaphore.clone().try_acquire_owned().map_err(|_| {
            RuntimeError::SubAgentError("maximum concurrent subagents reached".into())
        })?;

        let handle = Arc::new(SubAgentHandle::new(task, parent_id, depth, Some(permit)));

        // Store in active map
        self.active
            .write()
            .await
            .insert(handle.id.clone(), handle.clone());

        Ok(handle)
    }

    /// Release a subagent from the active pool and move to history.
    ///
    /// Releases the semaphore permit so another sub-agent can be spawned.
    /// The handle's status should already be set (completed/failed/timed out)
    /// before calling this.
    pub async fn release(&self, id: &SubAgentId) {
        let mut active = self.active.write().await;
        if let Some(handle) = active.remove(id) {
            handle.release_permit();
            self.push_to_history(handle).await;
            if active.is_empty() {
                self.completion_notify.notify_waiters();
            }
        }
    }

    /// Stop a specific subagent: cancel it and move to history.
    pub async fn stop(&self, id: &SubAgentId) -> Option<Arc<SubAgentHandle>> {
        let mut active = self.active.write().await;
        if let Some(handle) = active.remove(id) {
            handle.cancel().await;
            handle.release_permit();
            self.push_to_history(handle.clone()).await;
            if active.is_empty() {
                self.completion_notify.notify_waiters();
            }
            Some(handle)
        } else {
            None
        }
    }

    /// Push a handle to history with FIFO eviction at capacity.
    async fn push_to_history(&self, handle: Arc<SubAgentHandle>) {
        let mut completed = self.completed.write().await;
        if completed.len() >= self.max_history {
            completed.remove(0);
        }
        completed.push(handle);
    }

    /// Get active subagent by ID.
    pub async fn get(&self, id: &SubAgentId) -> Option<Arc<SubAgentHandle>> {
        self.active.read().await.get(id).cloned()
    }

    /// List active subagents.
    pub async fn list_active(&self) -> Vec<Arc<SubAgentHandle>> {
        self.active.read().await.values().cloned().collect()
    }

    /// Get count of active subagents.
    pub async fn active_count(&self) -> usize {
        self.active.read().await.len()
    }

    /// Get available capacity.
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Check if a child can be spawned for the given parent without actually spawning.
    ///
    /// Returns `true` if both depth limit and concurrency permit are available.
    pub async fn can_spawn_child(&self, parent_id: &SubAgentId) -> bool {
        let active = self.active.read().await;
        let parent_depth = if let Some(parent) = active.get(parent_id) {
            parent.depth
        } else {
            return false; // parent not found
        };

        let Some(child_depth) = parent_depth.checked_add(1) else {
            return false;
        };
        if child_depth >= self.max_depth {
            return false;
        }

        self.semaphore.available_permits() > 0
    }

    /// Get the cancellation token for cooperative cancellation.
    ///
    /// Sub-agent executors should select on this token in their run loop.
    /// Cancelling this token signals all sub-agents to stop cooperatively.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    /// Cancel all active subagents and move them to history.
    ///
    /// Also cancels the cooperative cancellation token so in-flight sub-agents
    /// can observe the cancellation and stop gracefully.
    pub async fn cancel_all(&self) {
        // Signal cooperative cancellation to all sub-agents.
        self.cancellation_token.cancel();

        let mut active = self.active.write().await;
        let handles: Vec<Arc<SubAgentHandle>> = active.drain().map(|(_, h)| h).collect();
        drop(active);

        for handle in handles {
            handle.cancel().await;
            handle.release_permit();
            self.push_to_history(handle).await;
        }

        self.completion_notify.notify_waiters();
    }

    /// Wait until the active pool is empty.
    pub async fn wait_for_completion(&self) {
        loop {
            if self.active.read().await.is_empty() {
                return;
            }
            self.completion_notify.notified().await;
        }
    }

    /// Wait until the active pool is empty, or the timeout expires.
    ///
    /// Returns `true` if the pool drained before the timeout, `false` otherwise.
    pub async fn wait_for_completion_timeout(&self, timeout: Duration) -> bool {
        tokio::select! {
            () = self.wait_for_completion() => true,
            () = tokio::time::sleep(timeout) => {
                self.active.read().await.is_empty()
            }
        }
    }

    /// Get direct children of a parent subagent (from both active and completed).
    pub async fn get_children(&self, parent_id: &SubAgentId) -> Vec<Arc<SubAgentHandle>> {
        let mut children = Vec::new();

        let active = self.active.read().await;
        for handle in active.values() {
            if handle.parent_id.as_ref() == Some(parent_id) {
                children.push(handle.clone());
            }
        }
        drop(active);

        let completed = self.completed.read().await;
        for handle in &*completed {
            if handle.parent_id.as_ref() == Some(parent_id) {
                children.push(handle.clone());
            }
        }

        children
    }

    /// Get all descendants of a parent subagent (BFS, from both active and completed).
    pub async fn get_subtree(&self, parent_id: &SubAgentId) -> Vec<Arc<SubAgentHandle>> {
        // Collect all handles into a single list for BFS
        let active = self.active.read().await;
        let completed = self.completed.read().await;

        let all_handles: Vec<Arc<SubAgentHandle>> = active
            .values()
            .cloned()
            .chain(completed.iter().cloned())
            .collect();
        drop(active);
        drop(completed);

        let mut result = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(parent_id.clone());

        while let Some(current_id) = queue.pop_front() {
            for handle in &all_handles {
                if handle.parent_id.as_ref() == Some(&current_id) {
                    result.push(handle.clone());
                    queue.push_back(handle.id.clone());
                }
            }
        }

        result
    }

    /// Cancel all active descendants of a parent subagent and move them to history.
    ///
    /// Returns the number of subagents cancelled.
    pub async fn cancel_subtree(&self, parent_id: &SubAgentId) -> usize {
        // First find all descendant IDs via BFS over the active pool
        let active = self.active.read().await;
        let mut to_cancel = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(parent_id.clone());

        while let Some(current_id) = queue.pop_front() {
            for (id, handle) in active.iter() {
                if handle.parent_id.as_ref() == Some(&current_id) {
                    to_cancel.push(id.clone());
                    queue.push_back(id.clone());
                }
            }
        }
        drop(active);

        let mut cancelled = 0usize;
        for id in &to_cancel {
            if self.stop(id).await.is_some() {
                cancelled = cancelled.saturating_add(1);
            }
        }
        cancelled
    }

    /// Get completed subagents history.
    pub async fn history(&self) -> Vec<Arc<SubAgentHandle>> {
        self.completed.read().await.clone()
    }

    /// Clear completed history.
    pub async fn clear_history(&self) {
        self.completed.write().await.clear();
    }

    /// Get pool statistics.
    pub async fn stats(&self) -> SubAgentPoolStats {
        let active = self.active.read().await;
        let completed = self.completed.read().await;

        let (succeeded, failed, cancelled, timed_out) =
            completed
                .iter()
                .fold(
                    (0usize, 0usize, 0usize, 0usize),
                    |(s, f, c, t), h| match h.final_status() {
                        Some(SubAgentStatus::Completed) => (s.saturating_add(1), f, c, t),
                        Some(SubAgentStatus::Failed) => (s, f.saturating_add(1), c, t),
                        Some(SubAgentStatus::Cancelled) => (s, f, c.saturating_add(1), t),
                        Some(SubAgentStatus::TimedOut) => (s, f, c, t.saturating_add(1)),
                        _ => (s, f, c, t),
                    },
                );

        SubAgentPoolStats {
            max_concurrent: self.max_concurrent,
            max_depth: self.max_depth,
            active: active.len(),
            available: self.semaphore.available_permits(),
            total_completed: completed.len(),
            succeeded,
            failed,
            cancelled,
            timed_out,
        }
    }
}

/// Statistics for a subagent pool.
#[derive(Debug, Clone)]
pub struct SubAgentPoolStats {
    /// Maximum concurrent subagents.
    pub max_concurrent: usize,
    /// Maximum nesting depth.
    pub max_depth: usize,
    /// Currently active subagents.
    pub active: usize,
    /// Available permits.
    pub available: usize,
    /// Total completed subagents.
    pub total_completed: usize,
    /// Successfully completed.
    pub succeeded: usize,
    /// Failed subagents.
    pub failed: usize,
    /// Cancelled subagents.
    pub cancelled: usize,
    /// Timed out subagents.
    pub timed_out: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_subagent_lifecycle() {
        let handle = SubAgentHandle::new("test task", None, 0, None);

        assert_eq!(handle.status().await, SubAgentStatus::Initializing);
        assert!(!handle.is_done().await);

        handle.mark_running().await;
        assert_eq!(handle.status().await, SubAgentStatus::Running);

        handle.complete("success").await;
        assert_eq!(handle.status().await, SubAgentStatus::Completed);
        assert!(handle.is_done().await);
        assert_eq!(handle.result().await, Some("success".into()));
    }

    #[tokio::test]
    async fn test_subagent_failure() {
        let handle = SubAgentHandle::new("test task", None, 0, None);

        handle.mark_running().await;
        handle.fail("something went wrong").await;

        assert_eq!(handle.status().await, SubAgentStatus::Failed);
        assert!(handle.is_done().await);
        assert_eq!(handle.error().await, Some("something went wrong".into()));
    }

    #[tokio::test]
    async fn test_subagent_final_status() {
        let handle = SubAgentHandle::new("test task", None, 0, None);
        assert_eq!(handle.final_status(), None);

        handle.mark_running().await;
        assert_eq!(handle.final_status(), None);

        handle.complete("done").await;
        assert_eq!(handle.final_status(), Some(SubAgentStatus::Completed));
    }

    #[tokio::test]
    async fn test_pool_spawn() {
        let pool = SubAgentPool::new(5, 3);

        let handle = pool.spawn("task 1", None).await.unwrap();
        assert_eq!(pool.active_count().await, 1);

        pool.release(&handle.id).await;
        assert_eq!(pool.active_count().await, 0);
        assert_eq!(pool.history().await.len(), 1);
    }

    #[tokio::test]
    async fn test_pool_max_depth() {
        let pool = SubAgentPool::new(5, 2);

        // Depth 0
        let h1 = pool.spawn("task 1", None).await.unwrap();
        assert_eq!(h1.depth, 0);

        // Depth 1
        let h2 = pool.spawn("task 2", Some(h1.id.clone())).await.unwrap();
        assert_eq!(h2.depth, 1);

        // Depth 2 should fail (max_depth = 2 means 0 and 1 only)
        let result = pool.spawn("task 3", Some(h2.id.clone())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pool_cancel_all() {
        let pool = SubAgentPool::new(5, 3);

        let h1 = pool.spawn("task 1", None).await.unwrap();
        let h2 = pool.spawn("task 2", None).await.unwrap();

        pool.cancel_all().await;

        assert_eq!(h1.status().await, SubAgentStatus::Cancelled);
        assert_eq!(h2.status().await, SubAgentStatus::Cancelled);
        // Handles should be moved to history
        assert_eq!(pool.active_count().await, 0);
        assert_eq!(pool.history().await.len(), 2);
    }

    #[tokio::test]
    async fn test_semaphore_limits_concurrency() {
        let pool = SubAgentPool::new(2, 5);

        let _h1 = pool.spawn("task 1", None).await.unwrap();
        let _h2 = pool.spawn("task 2", None).await.unwrap();

        // Third spawn should fail — semaphore exhausted
        let result = pool.spawn("task 3", None).await;
        assert!(result.is_err());
        assert_eq!(pool.available_permits(), 0);
    }

    #[tokio::test]
    async fn test_permit_released_on_complete() {
        let pool = SubAgentPool::new(1, 5);

        let h1 = pool.spawn("task 1", None).await.unwrap();
        assert_eq!(pool.available_permits(), 0);

        // Completing moves handle to history, dropping the permit
        pool.release(&h1.id).await;
        assert_eq!(pool.available_permits(), 1);

        // Should be able to spawn again
        let _h2 = pool.spawn("task 2", None).await.unwrap();
        assert_eq!(pool.available_permits(), 0);
    }

    #[tokio::test]
    async fn test_stop_cancels_and_moves_to_history() {
        let pool = SubAgentPool::new(5, 3);

        let h = pool.spawn("task 1", None).await.unwrap();
        let id = h.id.clone();

        let stopped = pool.stop(&id).await;
        assert!(stopped.is_some());

        let handle = stopped.unwrap();
        assert_eq!(handle.status().await, SubAgentStatus::Cancelled);
        assert_eq!(pool.active_count().await, 0);
        assert_eq!(pool.history().await.len(), 1);
    }

    #[tokio::test]
    async fn test_stop_nonexistent_returns_none() {
        let pool = SubAgentPool::new(5, 3);
        let result = pool.stop(&SubAgentId::new()).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_can_spawn_child_checks_depth_and_concurrency() {
        let pool = SubAgentPool::new(3, 2);

        // Depth 0
        let h1 = pool.spawn("task 1", None).await.unwrap();
        assert!(pool.can_spawn_child(&h1.id).await);

        // Depth 1 (max_depth=2, so depth 1 is allowed but children of depth 1 are not)
        let h2 = pool.spawn("task 2", Some(h1.id.clone())).await.unwrap();
        assert!(!pool.can_spawn_child(&h2.id).await); // depth 2 >= max_depth

        // Non-existent parent
        assert!(!pool.can_spawn_child(&SubAgentId::new()).await);
    }

    #[tokio::test]
    async fn test_can_spawn_child_checks_concurrency() {
        let pool = SubAgentPool::new(2, 5);

        let h1 = pool.spawn("task 1", None).await.unwrap();
        let _h2 = pool.spawn("task 2", None).await.unwrap();

        // No permits left
        assert!(!pool.can_spawn_child(&h1.id).await);
    }

    #[tokio::test]
    async fn test_wait_for_completion_returns_when_empty() {
        let pool = Arc::new(SubAgentPool::new(5, 3));

        // Empty pool — should return immediately
        pool.wait_for_completion().await;

        let h = pool.spawn("task 1", None).await.unwrap();
        let h_id = h.id.clone();
        let pool_clone = pool.clone();

        let waiter = tokio::spawn(async move {
            pool_clone.wait_for_completion().await;
        });

        // Give the waiter a moment to register
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!waiter.is_finished());

        pool.release(&h_id).await;

        // Waiter should finish now
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter should complete")
            .expect("waiter should not panic");
    }

    #[tokio::test]
    async fn test_wait_for_completion_timeout_returns_false_on_expiry() {
        let pool = SubAgentPool::new(5, 3);

        let _h = pool.spawn("task 1", None).await.unwrap();

        let drained = pool
            .wait_for_completion_timeout(Duration::from_millis(50))
            .await;
        assert!(!drained);
    }

    #[tokio::test]
    async fn test_wait_for_completion_timeout_returns_true_on_drain() {
        let pool = Arc::new(SubAgentPool::new(5, 3));

        let h = pool.spawn("task 1", None).await.unwrap();
        let h_id = h.id.clone();
        let pool_clone = pool.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            pool_clone.release(&h_id).await;
        });

        let drained = pool
            .wait_for_completion_timeout(Duration::from_secs(2))
            .await;
        assert!(drained);
    }

    #[tokio::test]
    async fn test_get_children_returns_direct_children_only() {
        let pool = SubAgentPool::new(10, 5);

        let root = pool.spawn("root", None).await.unwrap();
        let child1 = pool.spawn("child1", Some(root.id.clone())).await.unwrap();
        let child2 = pool.spawn("child2", Some(root.id.clone())).await.unwrap();
        let _grandchild = pool
            .spawn("grandchild", Some(child1.id.clone()))
            .await
            .unwrap();

        let children = pool.get_children(&root.id).await;
        assert_eq!(children.len(), 2);

        let child_ids: Vec<_> = children.iter().map(|c| c.id.clone()).collect();
        assert!(child_ids.contains(&child1.id));
        assert!(child_ids.contains(&child2.id));
    }

    #[tokio::test]
    async fn test_get_children_includes_completed() {
        let pool = SubAgentPool::new(10, 5);

        let root = pool.spawn("root", None).await.unwrap();
        let child = pool.spawn("child", Some(root.id.clone())).await.unwrap();
        let child_id = child.id.clone();

        // Move child to completed
        pool.release(&child_id).await;

        let children = pool.get_children(&root.id).await;
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, child_id);
    }

    #[tokio::test]
    async fn test_get_subtree_returns_all_descendants() {
        let pool = SubAgentPool::new(10, 5);

        let root = pool.spawn("root", None).await.unwrap();
        let child1 = pool.spawn("child1", Some(root.id.clone())).await.unwrap();
        let child2 = pool.spawn("child2", Some(root.id.clone())).await.unwrap();
        let grandchild1 = pool
            .spawn("grandchild1", Some(child1.id.clone()))
            .await
            .unwrap();
        let grandchild2 = pool
            .spawn("grandchild2", Some(child2.id.clone()))
            .await
            .unwrap();

        let subtree = pool.get_subtree(&root.id).await;
        assert_eq!(subtree.len(), 4);

        let ids: Vec<_> = subtree.iter().map(|h| h.id.clone()).collect();
        assert!(ids.contains(&child1.id));
        assert!(ids.contains(&child2.id));
        assert!(ids.contains(&grandchild1.id));
        assert!(ids.contains(&grandchild2.id));
        // Root itself should NOT be in the subtree
        assert!(!ids.contains(&root.id));
    }

    #[tokio::test]
    async fn test_cancel_subtree_cancels_only_descendants() {
        let pool = SubAgentPool::new(10, 5);

        let root = pool.spawn("root", None).await.unwrap();
        let child = pool.spawn("child", Some(root.id.clone())).await.unwrap();
        let grandchild = pool
            .spawn("grandchild", Some(child.id.clone()))
            .await
            .unwrap();
        let sibling = pool.spawn("sibling", None).await.unwrap();

        let cancelled = pool.cancel_subtree(&root.id).await;
        assert_eq!(cancelled, 2); // child + grandchild

        // Root should still be active
        assert!(pool.get(&root.id).await.is_some());
        // Child and grandchild should be in history
        assert!(pool.get(&child.id).await.is_none());
        assert!(pool.get(&grandchild.id).await.is_none());
        // Sibling should still be active
        assert!(pool.get(&sibling.id).await.is_some());
    }

    #[tokio::test]
    async fn test_stats_counts_are_accurate() {
        let pool = SubAgentPool::new(10, 5);

        let h1 = pool.spawn("task 1", None).await.unwrap();
        let h2 = pool.spawn("task 2", None).await.unwrap();
        let h3 = pool.spawn("task 3", None).await.unwrap();
        let h4 = pool.spawn("task 4", None).await.unwrap();

        // Complete one successfully
        h1.complete("result").await;
        pool.release(&h1.id).await;

        // Fail one
        h2.fail("error").await;
        pool.release(&h2.id).await;

        // Cancel one
        pool.stop(&h3.id).await;

        // Timeout one
        h4.timeout().await;
        pool.release(&h4.id).await;

        let stats = pool.stats().await;
        assert_eq!(stats.max_concurrent, 10);
        assert_eq!(stats.max_depth, 5);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.total_completed, 4);
        assert_eq!(stats.succeeded, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.cancelled, 1);
        assert_eq!(stats.timed_out, 1);
    }

    #[tokio::test]
    async fn test_stats_with_active() {
        let pool = SubAgentPool::new(5, 3);

        let _h1 = pool.spawn("task 1", None).await.unwrap();
        let h2 = pool.spawn("task 2", None).await.unwrap();
        h2.complete("done").await;
        pool.release(&h2.id).await;

        let stats = pool.stats().await;
        assert_eq!(stats.active, 1);
        assert_eq!(stats.total_completed, 1);
        assert_eq!(stats.succeeded, 1);
        assert_eq!(stats.available, 4);
    }

    #[tokio::test]
    async fn test_history_eviction_at_capacity() {
        let pool = SubAgentPool::with_max_history(10, 5, 3);

        // Spawn and release 5 subagents into a pool with max_history=3
        for i in 0..5 {
            let h = pool.spawn(format!("task {i}"), None).await.unwrap();
            h.complete(format!("result {i}")).await;
            pool.release(&h.id).await;
        }

        // Only the 3 most recent should be in history
        let history = pool.history().await;
        assert_eq!(history.len(), 3);
        // Oldest entry should be task 2 (tasks 0 and 1 were evicted)
        assert_eq!(history[0].task, "task 2");
        assert_eq!(history[1].task, "task 3");
        assert_eq!(history[2].task, "task 4");
    }
}
