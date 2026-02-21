use super::types::BudgetWarning;
use crate::budget::{BudgetResult, BudgetTracker, WorkspaceBudgetTracker};
use crate::error::ApprovalError;
use std::sync::Arc;

/// Ensures actions that charge money fall within configured user and workspace budgets.
pub struct BudgetValidator {
    /// Global or local session tracker for current spending limits.
    pub(crate) tracker: Arc<BudgetTracker>,
    /// Global or local workspace tracker for organizational limits.
    pub(crate) workspace_tracker: Option<Arc<WorkspaceBudgetTracker>>,
}

impl BudgetValidator {
    /// Creates a new `BudgetValidator`.
    pub fn new(
        tracker: Arc<BudgetTracker>,
        workspace_tracker: Option<Arc<WorkspaceBudgetTracker>>,
    ) -> Self {
        Self {
            tracker,
            workspace_tracker,
        }
    }

    /// Checks the overall workspace budget limits against a pending transactional cost.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested cost would breach the workspace budget limits.
    pub fn check_workspace_budget(
        &self,
        cost: f64,
    ) -> Result<Option<BudgetWarning>, ApprovalError> {
        let Some(ref ws_budget) = self.workspace_tracker else {
            return Ok(None);
        };
        match ws_budget.check_and_reserve(cost) {
            BudgetResult::Exceeded {
                reason,
                requested,
                available,
            } => Err(ApprovalError::Denied {
                reason: format!(
                    "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                ),
            }),
            BudgetResult::WarnAndAllow {
                current_spend,
                session_max,
                percent_used,
            } => Ok(Some(BudgetWarning {
                current_spend,
                session_max,
                percent_used,
            })),
            BudgetResult::Allowed => Ok(None),
        }
    }

    /// Checks the specific user's session budget against a pending transactional cost.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested cost would breach the session budget limits.
    pub fn check_session_budget(&self, cost: f64) -> Result<Option<BudgetWarning>, ApprovalError> {
        match self.tracker.check_and_reserve(cost) {
            BudgetResult::Exceeded {
                reason,
                requested,
                available,
            } => Err(ApprovalError::Denied {
                reason: format!(
                    "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                ),
            }),
            BudgetResult::WarnAndAllow {
                current_spend,
                session_max,
                percent_used,
            } => Ok(Some(BudgetWarning {
                current_spend,
                session_max,
                percent_used,
            })),
            BudgetResult::Allowed => Ok(None),
        }
    }
}
