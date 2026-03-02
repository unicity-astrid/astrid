//! Workspace-scoped helpers: KV namespacing, allowance/escape/budget persistence.

use astrid_approval::allowance::Allowance;
use tracing::warn;

use super::RpcImpl;
use crate::rpc::DaemonEvent;

/// Build a workspace-namespaced key for the KV store.
///
/// Uses the workspace UUID to namespace keys, e.g. `ws:<uuid>:allowances`.
pub(in crate::server) fn ws_ns(workspace_id: &uuid::Uuid, suffix: &str) -> String {
    format!("ws:{workspace_id}:{suffix}")
}

impl RpcImpl {
    /// Broadcast an event to all active sessions.
    ///
    /// Acquires a read lock on the session map (brief), iterates each
    /// session's `event_tx`, and sends the event. Must NOT be called while
    /// holding the plugin registry lock (deadlock risk).
    pub(super) async fn broadcast_to_all_sessions(&self, event: DaemonEvent) {
        let sessions = self.sessions.read().await;
        for handle in sessions.values() {
            let _ = handle.event_tx.send(event.clone());
        }
    }

    /// Load workspace-scoped allowances from the workspace KV store.
    pub(super) async fn load_workspace_allowances(&self) -> Vec<Allowance> {
        let ns = ws_ns(&self.workspace_id, "allowances");
        match self.workspace_kv.get(&ns, "all").await {
            Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Save workspace-scoped allowances to the workspace KV store.
    pub(super) async fn save_workspace_allowances(&self, allowances: &[Allowance]) {
        let ns = ws_ns(&self.workspace_id, "allowances");
        if let Ok(data) = serde_json::to_vec(allowances)
            && let Err(e) = self.workspace_kv.set(&ns, "all", data).await
        {
            warn!(error = %e, "Failed to save workspace allowances");
        }
    }

    /// Load workspace escape cache from the workspace KV store.
    pub(super) async fn load_workspace_escape(
        &self,
    ) -> Option<astrid_workspace::escape::EscapeState> {
        let ns = ws_ns(&self.workspace_id, "escape");
        match self.workspace_kv.get(&ns, "all").await {
            Ok(Some(data)) => serde_json::from_slice(&data).ok(),
            _ => None,
        }
    }

    /// Save workspace escape cache to the workspace KV store.
    pub(super) async fn save_workspace_escape(
        &self,
        state: &astrid_workspace::escape::EscapeState,
    ) {
        let ns = ws_ns(&self.workspace_id, "escape");
        if let Ok(data) = serde_json::to_vec(state)
            && let Err(e) = self.workspace_kv.set(&ns, "all", data).await
        {
            warn!(error = %e, "Failed to save workspace escape state");
        }
    }

    /// Save workspace cumulative budget snapshot to the workspace KV store.
    pub(super) async fn save_workspace_budget(&self) {
        let ns = ws_ns(&self.workspace_id, "budget");
        let snapshot = self.workspace_budget_tracker.snapshot();
        if let Ok(data) = serde_json::to_vec(&snapshot)
            && let Err(e) = self.workspace_kv.set(&ns, "all", data).await
        {
            warn!(error = %e, "Failed to save workspace budget");
        }
    }
}
