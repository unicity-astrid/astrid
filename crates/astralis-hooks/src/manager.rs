//! Hook manager - manages hook registration and triggering.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::executor::HookExecutor;
use crate::hook::{Hook, HookEvent};
use crate::result::{HookContext, HookExecution, HookResult};

/// Manages hooks and their execution.
#[derive(Debug)]
pub struct HookManager {
    /// Registered hooks, indexed by ID.
    hooks: Arc<RwLock<HashMap<Uuid, Hook>>>,
    /// Hooks grouped by event type.
    hooks_by_event: Arc<RwLock<HashMap<HookEvent, Vec<Uuid>>>>,
    /// The hook executor.
    executor: HookExecutor,
}

impl HookManager {
    /// Create a new hook manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hooks: Arc::new(RwLock::new(HashMap::new())),
            hooks_by_event: Arc::new(RwLock::new(HashMap::new())),
            executor: HookExecutor::new(),
        }
    }

    /// Register a hook.
    pub async fn register(&self, hook: Hook) {
        let hook_id = hook.id;
        let event = hook.event;

        info!(
            hook_id = %hook_id,
            hook_name = ?hook.name,
            event = %event,
            "Registering hook"
        );

        // Add to hooks map
        {
            let mut hooks = self.hooks.write().await;
            hooks.insert(hook_id, hook);
        }

        // Add to event index
        {
            let mut by_event = self.hooks_by_event.write().await;
            by_event.entry(event).or_default().push(hook_id);
        }
    }

    /// Register multiple hooks.
    pub async fn register_all(&self, hooks: Vec<Hook>) {
        for hook in hooks {
            self.register(hook).await;
        }
    }

    /// Unregister a hook by ID.
    pub async fn unregister(&self, hook_id: Uuid) -> Option<Hook> {
        info!(hook_id = %hook_id, "Unregistering hook");

        // Remove from hooks map
        let hook = {
            let mut hooks = self.hooks.write().await;
            hooks.remove(&hook_id)
        };

        // Remove from event index
        if let Some(ref hook) = hook {
            let mut by_event = self.hooks_by_event.write().await;
            if let Some(ids) = by_event.get_mut(&hook.event) {
                ids.retain(|id| *id != hook_id);
            }
        }

        hook
    }

    /// Enable a hook.
    pub async fn enable(&self, hook_id: Uuid) -> bool {
        let mut hooks = self.hooks.write().await;
        if let Some(hook) = hooks.get_mut(&hook_id) {
            hook.enabled = true;
            info!(hook_id = %hook_id, "Hook enabled");
            true
        } else {
            warn!(hook_id = %hook_id, "Hook not found");
            false
        }
    }

    /// Disable a hook.
    pub async fn disable(&self, hook_id: Uuid) -> bool {
        let mut hooks = self.hooks.write().await;
        if let Some(hook) = hooks.get_mut(&hook_id) {
            hook.enabled = false;
            info!(hook_id = %hook_id, "Hook disabled");
            true
        } else {
            warn!(hook_id = %hook_id, "Hook not found");
            false
        }
    }

    /// Get a hook by ID.
    pub async fn get(&self, hook_id: Uuid) -> Option<Hook> {
        let hooks = self.hooks.read().await;
        hooks.get(&hook_id).cloned()
    }

    /// Get all hooks.
    pub async fn all(&self) -> Vec<Hook> {
        let hooks = self.hooks.read().await;
        hooks.values().cloned().collect()
    }

    /// Get all hooks for an event.
    pub async fn hooks_for_event(&self, event: HookEvent) -> Vec<Hook> {
        let by_event = self.hooks_by_event.read().await;
        let hooks = self.hooks.read().await;

        let hook_ids = by_event.get(&event).cloned().unwrap_or_default();
        let mut result: Vec<Hook> = hook_ids
            .iter()
            .filter_map(|id| hooks.get(id).cloned())
            .collect();

        // Sort by priority (lower first)
        result.sort_by_key(|h| h.priority);

        result
    }

    /// Trigger all hooks for an event.
    ///
    /// Returns the executions and the combined result.
    pub async fn trigger(
        &self,
        event: HookEvent,
        context: HookContext,
    ) -> (Vec<HookExecution>, HookResult) {
        debug!(event = %event, "Triggering hooks");

        let hooks = self.hooks_for_event(event).await;

        if hooks.is_empty() {
            debug!(event = %event, "No hooks registered for event");
            return (Vec::new(), HookResult::Continue);
        }

        info!(
            event = %event,
            hook_count = hooks.len(),
            "Executing hooks for event"
        );

        let executions = self.executor.execute_all(&hooks, context).await;
        let combined = HookExecutor::combine_results(&executions);

        (executions, combined)
    }

    /// Trigger hooks and return only the combined result.
    pub async fn trigger_simple(&self, event: HookEvent, context: HookContext) -> HookResult {
        let (_, result) = self.trigger(event, context).await;
        result
    }

    /// Get statistics about registered hooks.
    pub async fn stats(&self) -> HookStats {
        let hooks = self.hooks.read().await;
        let by_event = self.hooks_by_event.read().await;

        let total = hooks.len();
        let enabled = hooks.values().filter(|h| h.enabled).count();
        let events_with_hooks = by_event.iter().filter(|(_, ids)| !ids.is_empty()).count();

        HookStats {
            total,
            enabled,
            disabled: total - enabled,
            events_with_hooks,
        }
    }

    /// Clear all hooks.
    pub async fn clear(&self) {
        info!("Clearing all hooks");
        self.hooks.write().await.clear();
        self.hooks_by_event.write().await.clear();
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about registered hooks.
#[derive(Debug, Clone)]
pub struct HookStats {
    /// Total number of hooks.
    pub total: usize,
    /// Number of enabled hooks.
    pub enabled: usize,
    /// Number of disabled hooks.
    pub disabled: usize,
    /// Number of events that have hooks.
    pub events_with_hooks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookHandler;

    #[tokio::test]
    async fn test_manager_register() {
        let manager = HookManager::new();
        let hook = Hook::new(HookEvent::SessionStart).with_name("test-hook");

        manager.register(hook.clone()).await;

        let retrieved = manager.get(hook.id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, Some("test-hook".to_string()));
    }

    #[tokio::test]
    async fn test_manager_unregister() {
        let manager = HookManager::new();
        let hook = Hook::new(HookEvent::SessionStart);
        let hook_id = hook.id;

        manager.register(hook).await;
        assert!(manager.get(hook_id).await.is_some());

        let removed = manager.unregister(hook_id).await;
        assert!(removed.is_some());
        assert!(manager.get(hook_id).await.is_none());
    }

    #[tokio::test]
    async fn test_manager_enable_disable() {
        let manager = HookManager::new();
        let hook = Hook::new(HookEvent::SessionStart);
        let hook_id = hook.id;

        manager.register(hook).await;

        manager.disable(hook_id).await;
        let hook = manager.get(hook_id).await.unwrap();
        assert!(!hook.enabled);

        manager.enable(hook_id).await;
        let hook = manager.get(hook_id).await.unwrap();
        assert!(hook.enabled);
    }

    #[tokio::test]
    async fn test_manager_hooks_for_event() {
        let manager = HookManager::new();

        let hook1 = Hook::new(HookEvent::PreToolCall).with_priority(10);
        let hook2 = Hook::new(HookEvent::PreToolCall).with_priority(5);
        let hook3 = Hook::new(HookEvent::PostToolCall);

        manager.register(hook1).await;
        manager.register(hook2).await;
        manager.register(hook3).await;

        let pre_tool_hooks = manager.hooks_for_event(HookEvent::PreToolCall).await;
        assert_eq!(pre_tool_hooks.len(), 2);
        // Should be sorted by priority
        assert_eq!(pre_tool_hooks[0].priority, 5);
        assert_eq!(pre_tool_hooks[1].priority, 10);

        let post_tool_hooks = manager.hooks_for_event(HookEvent::PostToolCall).await;
        assert_eq!(post_tool_hooks.len(), 1);
    }

    #[tokio::test]
    async fn test_manager_trigger() {
        let manager = HookManager::new();

        let hook = Hook::new(HookEvent::SessionStart)
            .with_handler(HookHandler::Command {
                command: "echo".to_string(),
                args: vec!["continue".to_string()],
                env: Default::default(),
                working_dir: None,
            })
            .with_timeout(5);

        manager.register(hook).await;

        let context = HookContext::new(HookEvent::SessionStart);
        let (executions, result) = manager.trigger(HookEvent::SessionStart, context).await;

        assert_eq!(executions.len(), 1);
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn test_manager_stats() {
        let manager = HookManager::new();

        manager.register(Hook::new(HookEvent::SessionStart)).await;
        manager
            .register(Hook::new(HookEvent::SessionEnd).disabled())
            .await;
        manager.register(Hook::new(HookEvent::PreToolCall)).await;

        let stats = manager.stats().await;
        assert_eq!(stats.total, 3);
        assert_eq!(stats.enabled, 2);
        assert_eq!(stats.disabled, 1);
        assert_eq!(stats.events_with_hooks, 3);
    }
}
