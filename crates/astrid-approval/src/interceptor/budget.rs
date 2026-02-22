use super::types::BudgetWarning;
use crate::budget::{BudgetResult, BudgetTracker, WorkspaceBudgetTracker};
use crate::error::ApprovalError;
use std::sync::Arc;

/// An RAII guard representing a reserved budget amount.
/// If dropped without being explicitly committed, the reserved budget is automatically refunded.
#[must_use = "If you drop this reservation without committing, the budget is refunded."]
#[derive(Debug)]
pub struct BudgetReservation {
    tracker: Arc<BudgetTracker>,
    workspace_tracker: Option<Arc<WorkspaceBudgetTracker>>,
    cost: f64,
    committed: bool,
    warning: Option<BudgetWarning>,
}

impl BudgetReservation {
    /// Consumes the reservation and makes the budget deduction permanent.
    pub fn commit(mut self) {
        self.committed = true;
    }

    /// Gets the warning associated with the reservation, if any.
    #[must_use]
    pub fn warning(&self) -> Option<&BudgetWarning> {
        self.warning.as_ref()
    }
}

impl Drop for BudgetReservation {
    fn drop(&mut self) {
        if !self.committed {
            if let Some(ref ws_budget) = self.workspace_tracker {
                ws_budget.refund_cost(self.cost);
            }
            self.tracker.refund_cost(self.cost);
        }
    }
}

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
    pub fn check_and_reserve(&self, cost: f64) -> Result<BudgetReservation, ApprovalError> {
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
                Ok(BudgetReservation {
                    tracker: self.tracker.clone(),
                    workspace_tracker: self.workspace_tracker.clone(),
                    cost,
                    committed: false,
                    warning: Some(BudgetWarning {
                        current_spend,
                        session_max,
                        percent_used,
                    }),
                })
            },
            BudgetResult::Allowed => Ok(BudgetReservation {
                tracker: self.tracker.clone(),
                workspace_tracker: self.workspace_tracker.clone(),
                cost,
                committed: false,
                warning,
            }),
        }
    }
}
