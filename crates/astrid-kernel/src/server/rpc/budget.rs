//! Budget, allowance, and audit RPC method implementations.

use astrid_core::SessionId;
use chrono::DateTime;
use jsonrpsee::types::ErrorObjectOwned;

use super::RpcImpl;
use crate::rpc::{AllowanceInfo, AuditEntryInfo, BudgetInfo, error_codes};

impl RpcImpl {
    pub(super) async fn session_budget_impl(
        &self,
        session_id: SessionId,
    ) -> Result<BudgetInfo, ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            let h = sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?;

            if h.is_connector() {
                return Err(ErrorObjectOwned::owned(
                    error_codes::INVALID_REQUEST,
                    "session is managed by the inbound router and its budget cannot be queried via RPC",
                    None::<()>,
                ));
            }
            h
        };

        let session = handle.session.lock().await;
        let budget = &session.budget_tracker;

        let (workspace_spent, workspace_max, workspace_remaining) =
            if let Some(ref ws_budget) = session.workspace_budget_tracker {
                (
                    Some(ws_budget.spent()),
                    ws_budget.remaining().map(|r| r + ws_budget.spent()),
                    ws_budget.remaining(),
                )
            } else {
                (None, None, None)
            };

        Ok(BudgetInfo {
            session_spent_usd: budget.spent(),
            session_max_usd: budget.config().session_max_usd,
            session_remaining_usd: budget.remaining(),
            per_action_max_usd: budget.config().per_action_max_usd,
            warn_at_percent: budget.config().warn_at_percent,
            workspace_spent_usd: workspace_spent,
            workspace_max_usd: workspace_max,
            workspace_remaining_usd: workspace_remaining,
        })
    }

    pub(super) async fn session_allowances_impl(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<AllowanceInfo>, ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            let h = sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?;

            if h.is_connector() {
                return Err(ErrorObjectOwned::owned(
                    error_codes::INVALID_REQUEST,
                    "session is managed by the inbound router and its allowances cannot be queried via RPC",
                    None::<()>,
                ));
            }
            h
        };

        let session = handle.session.lock().await;
        let mut infos = Vec::new();

        for allowance in session.allowance_store.export_session_allowances() {
            infos.push(AllowanceInfo {
                id: allowance.id.to_string(),
                pattern: format!("{:?}", allowance.action_pattern),
                session_only: allowance.session_only,
                created_at: DateTime::from(allowance.created_at),
                expires_at: allowance.expires_at.map(DateTime::from),
                uses_remaining: allowance.uses_remaining,
            });
        }

        for allowance in session.allowance_store.export_workspace_allowances() {
            infos.push(AllowanceInfo {
                id: allowance.id.to_string(),
                pattern: format!("{:?}", allowance.action_pattern),
                session_only: allowance.session_only,
                created_at: DateTime::from(allowance.created_at),
                expires_at: allowance.expires_at.map(DateTime::from),
                uses_remaining: allowance.uses_remaining,
            });
        }

        Ok(infos)
    }

    pub(super) fn session_audit_impl(
        &self,
        session_id: &SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<AuditEntryInfo>, ErrorObjectOwned> {
        let entries = self
            .runtime
            .audit()
            .get_session_entries(session_id)
            .map_err(|e| {
                ErrorObjectOwned::owned(
                    error_codes::INTERNAL_ERROR,
                    format!("Failed to query audit log: {e}"),
                    None::<()>,
                )
            })?;

        let limit = limit.unwrap_or(20);
        let start = entries.len().saturating_sub(limit);

        Ok(entries[start..]
            .iter()
            .map(|entry| AuditEntryInfo {
                timestamp: DateTime::from(entry.timestamp),
                action: format!("{:?}", entry.action),
                outcome: match &entry.outcome {
                    astrid_audit::AuditOutcome::Success { details } => {
                        if let Some(d) = details {
                            format!("OK: {d}")
                        } else {
                            "OK".to_string()
                        }
                    },
                    astrid_audit::AuditOutcome::Failure { error } => {
                        format!("FAIL: {error}")
                    },
                },
            })
            .collect())
    }
}
