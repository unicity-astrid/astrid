use std::sync::Arc;
use crate::error::ApprovalError;
use crate::budget::{BudgetTracker, BudgetResult, WorkspaceBudgetTracker};
use super::types::BudgetWarning;

pub struct BudgetValidator {
    pub tracker: Arc<BudgetTracker>,
    pub workspace_tracker: Option<Arc<WorkspaceBudgetTracker>>,
}

impl BudgetValidator {
    pub fn new(tracker: Arc<BudgetTracker>, workspace_tracker: Option<Arc<WorkspaceBudgetTracker>>) -> Self {
        Self { tracker, workspace_tracker }
    }

    pub fn check_workspace_budget(&self, cost: f64) -> Result<Option<BudgetWarning>, ApprovalError> {
        let Some(ref ws_budget) = self.workspace_tracker else {
            return Ok(None);
        };
        match ws_budget.check_and_reserve(cost) {
            BudgetResult::Exceeded { reason, requested, available } => Err(ApprovalError::Denied {
                reason: format!("budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"),
            }),
            BudgetResult::WarnAndAllow { current_spend, session_max, percent_used } => {
                Ok(Some(BudgetWarning { current_spend, session_max, percent_used }))
            },
            BudgetResult::Allowed => Ok(None),
        }
    }

    pub fn check_session_budget(&self, cost: f64) -> Result<Option<BudgetWarning>, ApprovalError> {
        match self.tracker.check_and_reserve(cost) {
            BudgetResult::Exceeded { reason, requested, available } => Err(ApprovalError::Denied {
                reason: format!("budget exceeded ({reason}): requested ${requested:.2}, available ${available:.2}"),
            }),
            BudgetResult::WarnAndAllow { current_spend, session_max, percent_used } => {
                Ok(Some(BudgetWarning { current_spend, session_max, percent_used }))
            },
            BudgetResult::Allowed => Ok(None),
        }
    }
}
