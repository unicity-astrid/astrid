use serde::{Deserialize, Serialize};

/// Identifies the user and session that triggered the current capsule execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerContext {
    pub session_id: Option<String>,
    pub user_id: Option<String>,
}
