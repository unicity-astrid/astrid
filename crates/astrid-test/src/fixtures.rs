//! Test fixtures for common types.

use uuid::Uuid;

use astrid_core::{
    AgentId, ApprovalRequest, ElicitationRequest, ElicitationSchema, RiskLevel, SessionId,
};

/// Create a test agent ID.
#[must_use]
pub fn test_agent_id() -> AgentId {
    AgentId::new()
}

/// Create a test agent ID with a specific UUID.
#[must_use]
pub fn test_agent_id_from(uuid: Uuid) -> AgentId {
    AgentId::from_uuid(uuid)
}

/// Create a test session ID.
#[must_use]
pub fn test_session_id() -> SessionId {
    SessionId::new()
}

/// Create a test session ID with a specific UUID.
#[must_use]
pub fn test_session_id_from(uuid: Uuid) -> SessionId {
    SessionId::from_uuid(uuid)
}

/// Create a test approval request with default values.
#[must_use]
pub fn test_approval_request() -> ApprovalRequest {
    ApprovalRequest::new("test_operation", "This is a test operation")
        .with_risk_level(RiskLevel::Medium)
}

/// Create a test approval request for a specific operation.
#[must_use]
pub fn test_approval_request_for(
    operation: impl Into<String>,
    description: impl Into<String>,
) -> ApprovalRequest {
    ApprovalRequest::new(operation, description)
}

/// Create a high-risk approval request.
#[must_use]
pub fn test_high_risk_approval() -> ApprovalRequest {
    ApprovalRequest::new("delete_files", "Delete multiple files from the system")
        .with_risk_level(RiskLevel::High)
        .with_resource("/home/user/important")
}

/// Create a test elicitation request with default values.
#[must_use]
pub fn test_elicitation_request() -> ElicitationRequest {
    ElicitationRequest::new("test-server", "Please provide input")
}

/// Create a text elicitation request.
#[must_use]
pub fn test_text_elicitation(message: impl Into<String>) -> ElicitationRequest {
    ElicitationRequest::new("test-server", message).with_schema(ElicitationSchema::Text {
        placeholder: Some("Enter text...".to_string()),
        max_length: Some(1000),
    })
}

/// Create a secret elicitation request.
#[must_use]
pub fn test_secret_elicitation(message: impl Into<String>) -> ElicitationRequest {
    ElicitationRequest::new("test-server", message).with_schema(ElicitationSchema::Secret {
        placeholder: Some("Enter secret...".to_string()),
    })
}

/// Create a confirmation elicitation request.
#[must_use]
pub fn test_confirm_elicitation(message: impl Into<String>) -> ElicitationRequest {
    ElicitationRequest::new("test-server", message)
        .with_schema(ElicitationSchema::Confirm { default: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_fixture() {
        let id1 = test_agent_id();
        let id2 = test_agent_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_session_id_fixture() {
        let id = test_session_id();
        assert!(id.to_string().starts_with("session:"));
    }

    #[test]
    fn test_approval_request_fixture() {
        let req = test_approval_request();
        assert_eq!(req.operation, "test_operation");
        assert_eq!(req.risk_level, RiskLevel::Medium);
    }

    #[test]
    fn test_high_risk_approval_fixture() {
        let req = test_high_risk_approval();
        assert_eq!(req.risk_level, RiskLevel::High);
        assert!(req.resource.is_some());
    }

    #[test]
    fn test_elicitation_fixtures() {
        let text = test_text_elicitation("Enter name");
        assert!(matches!(text.schema, ElicitationSchema::Text { .. }));

        let secret = test_secret_elicitation("Enter password");
        assert!(matches!(secret.schema, ElicitationSchema::Secret { .. }));

        let confirm = test_confirm_elicitation("Are you sure?");
        assert!(matches!(confirm.schema, ElicitationSchema::Confirm { .. }));
    }
}
