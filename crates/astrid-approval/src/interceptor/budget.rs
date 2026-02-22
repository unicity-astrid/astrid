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

    /// Atomically checks and reserves cost from both workspace and session budgets.
    ///
    /// It first verifies if the budgets can accommodate the cost without reserving.
    /// If both pass, it then reserves the cost. If the session reservation fails after
    /// the workspace reservation succeeds, it rolls back the workspace reservation
    /// to prevent a resource leak.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested cost would breach either budget limits.
    pub fn check_and_reserve(&self, cost: f64) -> Result<Option<BudgetWarning>, ApprovalError> {
        // Step 1: Pre-check both budgets without reserving
        if let Some(ref ws_budget) = self.workspace_tracker
            && let BudgetResult::Exceeded {
                reason,
                requested,
                available,
            } = ws_budget.check_budget(cost)
        {
            return Err(ApprovalError::Denied {
                reason: format!(
                    "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                ),
            });
        }

        if let BudgetResult::Exceeded {
            reason,
            requested,
            available,
        } = self.tracker.check_budget(cost)
        {
            return Err(ApprovalError::Denied {
                reason: format!(
                    "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                ),
            });
        }

        // Step 2: Reserve on both, with rollback if session fails
        let mut warning = None;

        if let Some(ref ws_budget) = self.workspace_tracker {
            match ws_budget.check_and_reserve(cost) {
                BudgetResult::Exceeded {
                    reason,
                    requested,
                    available,
                } => {
                    return Err(ApprovalError::Denied {
                        reason: format!(
                            "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                        ),
                    });
                },
                BudgetResult::WarnAndAllow {
                    current_spend,
                    session_max,
                    percent_used,
                } => {
                    warning = Some(BudgetWarning {
                        current_spend,
                        session_max,
                        percent_used,
                    });
                },
                BudgetResult::Allowed => {},
            }
        }

        match self.tracker.check_and_reserve(cost) {
            BudgetResult::Exceeded {
                reason,
                requested,
                available,
            } => {
                // Rollback workspace budget to prevent a leak!
                if let Some(ref ws_budget) = self.workspace_tracker {
                    ws_budget.refund_cost(cost);
                }
                Err(ApprovalError::Denied {
                    reason: format!(
                        "budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"
                    ),
                })
            },
            BudgetResult::WarnAndAllow {
                current_spend,
                session_max,
                percent_used,
            } => {
                // Session warning is typically more immediate to the user, overwrite workspace warning if any
                Ok(Some(BudgetWarning {
                    current_spend,
                    session_max,
                    percent_used,
                }))
            },
            BudgetResult::Allowed => Ok(warning),
        }
    }

    /// Refunds a previously reserved cost to both workspace and session budgets.
    pub fn refund(&self, cost: f64) {
        if let Some(ref ws_budget) = self.workspace_tracker {
            ws_budget.refund_cost(cost);
        }
        self.tracker.refund_cost(cost);
    }
}
