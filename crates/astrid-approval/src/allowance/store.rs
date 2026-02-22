//! In-memory store for active allowances.

use crate::error::{ApprovalError, ApprovalResult};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::RwLock;

use super::{Allowance, AllowanceId};
use crate::action::SensitiveAction;

/// In-memory store for active allowances.
///
/// Thread-safe via internal [`RwLock`]. Supports pattern-based matching,
/// use tracking, expiration cleanup, and session clearing.
///
/// # Example
///
/// ```
/// use astrid_approval::AllowanceStore;
///
/// let store = AllowanceStore::new();
/// assert_eq!(store.count(), 0);
/// ```
pub struct AllowanceStore {
    allowances: RwLock<HashMap<AllowanceId, Allowance>>,
}

impl AllowanceStore {
    /// Create a new empty allowance store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            allowances: RwLock::new(HashMap::new()),
        }
    }

    /// Add an allowance to the store.
    ///
    /// # Errors
    ///
    /// Returns a storage error if the internal lock is poisoned.
    pub fn add_allowance(&self, allowance: Allowance) -> ApprovalResult<()> {
        let mut store = self
            .allowances
            .write()
            .map_err(|e| ApprovalError::Storage(e.to_string()))?;
        store.insert(allowance.id.clone(), allowance);
        Ok(())
    }

    /// Find the first valid allowance that matches an action.
    ///
    /// An allowance matches when:
    /// 1. Its pattern covers the action
    /// 2. It has not expired
    /// 3. It has uses remaining (if limited)
    /// 4. For workspace-scoped allowances (`workspace_root: Some(..)`),
    ///    the allowance's `workspace_root` must match the current `workspace_root`
    ///
    /// Returns a clone of the matching allowance, or `None`.
    #[must_use]
    pub fn find_matching(
        &self,
        action: &SensitiveAction,
        workspace_root: Option<&Path>,
    ) -> Option<Allowance> {
        let store = self.allowances.read().unwrap_or_else(|e| {
            tracing::warn!("AllowanceStore read lock poisoned, recovering");
            e.into_inner()
        });
        store
            .values()
            .find(|a| {
                if !a.is_valid() {
                    return false;
                }
                // Workspace-scoped allowances only match when the workspace root matches
                if let Some(allowance_ws) = &a.workspace_root
                    && workspace_root != Some(allowance_ws.as_path())
                {
                    return false;
                }
                a.action_pattern.matches(action, workspace_root)
            })
            .cloned()
    }

    /// Atomically find a matching allowance and consume one use.
    ///
    /// This combines [`find_matching`](Self::find_matching) and
    /// [`consume_use`](Self::consume_use) under a single write lock to prevent
    /// race conditions where two concurrent callers both find the same
    /// single-use allowance.
    ///
    /// Also cleans up expired allowances while the lock is held.
    ///
    /// Returns a clone of the matching allowance (before consumption), or `None`.
    #[must_use]
    pub fn find_matching_and_consume(
        &self,
        action: &SensitiveAction,
        workspace_root: Option<&Path>,
    ) -> Option<Allowance> {
        let mut store = self.allowances.write().unwrap_or_else(|e| {
            tracing::warn!("AllowanceStore lock poisoned, recovering");
            e.into_inner()
        });
        // Clean expired while we hold the lock
        store.retain(|_, a| !a.is_expired());
        let id = store
            .values()
            .find(|a| {
                a.is_valid()
                    && match &a.workspace_root {
                        Some(ws) => workspace_root == Some(ws.as_path()),
                        None => true,
                    }
                    && a.action_pattern.matches(action, workspace_root)
            })?
            .id
            .clone();
        let allowance = store.get(&id)?.clone();
        // Consume use atomically
        if let Some(remaining) = store.get_mut(&id).and_then(|a| a.uses_remaining.as_mut()) {
            *remaining = remaining.saturating_sub(1);
        }
        Some(allowance)
    }

    /// Consume one use of an allowance.
    ///
    /// For unlimited allowances (`uses_remaining: None`), this is a no-op.
    /// For limited allowances, decrements `uses_remaining` by 1.
    ///
    /// Returns `true` if the allowance still has uses remaining after consumption,
    /// `false` if this was the last use.
    ///
    /// # Errors
    ///
    /// Returns an error if the allowance is not found or the lock is poisoned.
    pub fn consume_use(&self, allowance_id: &AllowanceId) -> ApprovalResult<bool> {
        let mut store = self
            .allowances
            .write()
            .map_err(|e| ApprovalError::Storage(e.to_string()))?;

        let allowance = store.get_mut(allowance_id).ok_or_else(|| {
            ApprovalError::Storage(format!("allowance not found: {allowance_id}"))
        })?;

        if let Some(remaining) = &mut allowance.uses_remaining {
            *remaining = remaining.saturating_sub(1);
            Ok(*remaining > 0)
        } else {
            // Unlimited — always has uses remaining
            Ok(true)
        }
    }

    /// Remove all expired allowances from the store.
    ///
    /// Returns the number of allowances removed.
    pub fn cleanup_expired(&self) -> usize {
        let Ok(mut store) = self.allowances.write() else {
            return 0;
        };
        let before = store.len();
        store.retain(|_, a| !a.is_expired());
        before.saturating_sub(store.len())
    }

    /// Remove all session-only allowances from the store.
    ///
    /// Called when a session ends to clear temporary permissions.
    pub fn clear_session_allowances(&self) {
        if let Ok(mut store) = self.allowances.write() {
            store.retain(|_, a| !a.session_only);
        }
    }

    /// Get the number of allowances in the store.
    #[must_use]
    pub fn count(&self) -> usize {
        self.allowances.read().map(|s| s.len()).unwrap_or(0)
    }

    /// Export all session-scoped allowances for persistence.
    ///
    /// Returns a list of allowances that have `session_only: true`.
    /// These are the allowances that would be lost on restart without persistence.
    #[must_use]
    pub fn export_session_allowances(&self) -> Vec<Allowance> {
        self.allowances
            .read()
            .map(|store| {
                store
                    .values()
                    .filter(|a| a.session_only && a.is_valid())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Export all workspace-scoped allowances for persistence.
    ///
    /// Returns allowances that have `session_only: false` and a `workspace_root` set.
    /// These are the allowances that should be persisted in the workspace `state.db`.
    #[must_use]
    pub fn export_workspace_allowances(&self) -> Vec<Allowance> {
        self.allowances
            .read()
            .map(|store| {
                store
                    .values()
                    .filter(|a| !a.session_only && a.workspace_root.is_some() && a.is_valid())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Import allowances into the store, merging with existing ones.
    ///
    /// Used to restore session allowances from a persisted session.
    pub fn import_allowances(&self, allowances: Vec<Allowance>) {
        if let Ok(mut store) = self.allowances.write() {
            for allowance in allowances {
                if allowance.is_valid() {
                    store.insert(allowance.id.clone(), allowance);
                }
            }
        }
    }
}

impl Default for AllowanceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AllowanceStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.count();
        f.debug_struct("AllowanceStore")
            .field("count", &count)
            .finish()
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
