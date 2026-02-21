use async_trait::async_trait;

use super::error::FrontendResult;
use super::types::{
    ApprovalDecision, ApprovalRequest, ElicitationRequest, ElicitationResponse, FrontendContext,
    UrlElicitationRequest, UrlElicitationResponse, UserInput,
};
use crate::identity::{AstridUserId, FrontendType};
use crate::input::{MessageId, TaggedMessage};
use crate::verification::{VerificationRequest, VerificationResponse};

/// Frontend wrapper.
pub type ArcFrontend = std::sync::Arc<dyn Frontend + Send + Sync>;

/// The main frontend trait that all UI implementations must implement.
///
/// This trait defines the contract between the Astrid core and various
/// user interfaces (CLI, Discord, Web, etc.).
#[async_trait]
pub trait Frontend: Send + Sync {
    /// Get the current interaction context.
    ///
    /// This provides information about the current channel, user, and session.
    fn get_context(&self) -> FrontendContext;

    /// MCP elicitation - server asking user for input.
    ///
    /// This is used when an MCP server needs information from the user,
    /// such as API keys, preferences, or other configuration.
    async fn elicit(&self, request: ElicitationRequest) -> FrontendResult<ElicitationResponse>;

    /// URL-mode elicitation - OAuth, payments, etc.
    ///
    /// This presents a URL to the user for authentication flows,
    /// payment processing, or other web-based interactions.
    async fn elicit_url(
        &self,
        request: UrlElicitationRequest,
    ) -> FrontendResult<UrlElicitationResponse>;

    /// Request approval for sensitive operations.
    ///
    /// This is used for operations that require explicit user consent,
    /// such as file deletion, network access, or cost-incurring operations.
    async fn request_approval(&self, request: ApprovalRequest) -> FrontendResult<ApprovalDecision>;

    /// Display a status message to the user.
    fn show_status(&self, message: &str);

    /// Display an error message to the user.
    fn show_error(&self, error: &str);

    /// Notify that a tool call has started.
    ///
    /// `id` is the LLM-assigned call ID, `name` is the tool name,
    /// and `args` are the parsed tool arguments.
    fn tool_started(&self, _id: &str, _name: &str, _args: &serde_json::Value) {}

    /// Notify that a tool call has completed.
    ///
    /// `id` is the LLM-assigned call ID, `result` is the tool output,
    /// and `is_error` indicates whether the tool call failed.
    fn tool_completed(&self, _id: &str, _result: &str, _is_error: bool) {}

    /// Receive input from the user.
    ///
    /// Returns `None` if the user cancels or the input stream ends.
    async fn receive_input(&self) -> Option<UserInput>;

    /// Resolve a frontend user ID to an Astrid identity.
    ///
    /// Returns `None` if the user is not known.
    async fn resolve_identity(&self, frontend_user_id: &str) -> Option<AstridUserId>;

    /// Fetch a message by ID for verification.
    ///
    /// This is used to verify claims about what a user said.
    async fn get_message(&self, message_id: &MessageId) -> Option<TaggedMessage>;

    /// Send a verification request to a user.
    ///
    /// The method of delivery (inline buttons, DM, etc.) is determined
    /// by the frontend based on the risk level and context.
    async fn send_verification(
        &self,
        user_id: &str,
        request: VerificationRequest,
    ) -> FrontendResult<VerificationResponse>;

    /// Send an identity link code to a user.
    ///
    /// Used for cross-frontend identity linking.
    async fn send_link_code(&self, user_id: &str, code: &str) -> FrontendResult<()>;

    /// Get the frontend type.
    fn frontend_type(&self) -> FrontendType;
}

// ---------------------------------------------------------------------------
// Blanket adapter implementations for Frontend
// ---------------------------------------------------------------------------

#[async_trait]
impl<T: Frontend + ?Sized> crate::connector::ApprovalAdapter for T {
    async fn request_approval(
        &self,
        request: ApprovalRequest,
    ) -> crate::connector::ConnectorResult<ApprovalDecision> {
        Frontend::request_approval(self, request)
            .await
            .map_err(|e| crate::connector::ConnectorError::Internal(e.to_string()))
    }
}

#[async_trait]
impl<T: Frontend + ?Sized> crate::connector::ElicitationAdapter for T {
    async fn elicit(
        &self,
        request: ElicitationRequest,
    ) -> crate::connector::ConnectorResult<ElicitationResponse> {
        Frontend::elicit(self, request)
            .await
            .map_err(|e| crate::connector::ConnectorError::Internal(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::error::*;
    use crate::frontend::types::*;
    use crate::types::RiskLevel;
    use uuid::Uuid;

    #[test]
    fn test_frontend_user() {
        let user = FrontendUser::new("123")
            .with_display_name("Alice")
            .with_admin(true);

        assert_eq!(user.frontend_user_id, "123");
        assert_eq!(user.display_name, Some("Alice".to_string()));
        assert!(user.is_admin);
    }

    #[test]
    fn test_channel_info() {
        let dm = ChannelInfo::dm("dm_123");
        assert_eq!(dm.channel_type, ChannelType::DirectMessage);
        assert!(dm.guild_id.is_none());

        let guild = ChannelInfo::guild_channel("ch_456", "general", "guild_789");
        assert_eq!(guild.channel_type, ChannelType::GuildText);
        assert_eq!(guild.guild_id, Some("guild_789".to_string()));
    }

    #[test]
    fn test_elicitation_request() {
        let req = ElicitationRequest::new("test-server", "Enter your API key")
            .with_schema(ElicitationSchema::Secret { placeholder: None })
            .optional();

        assert_eq!(req.server_name, "test-server");
        assert!(!req.required);
    }

    #[test]
    fn test_approval_request() {
        let req = ApprovalRequest::new("delete_file", "Delete important.txt?")
            .with_risk_level(RiskLevel::High)
            .with_resource("/home/user/important.txt");

        assert_eq!(req.risk_level, RiskLevel::High);
        assert!(req.resource.is_some());
    }

    #[test]
    fn test_approval_decision() {
        let decision = ApprovalDecision::new(Uuid::new_v4(), ApprovalOption::AllowOnce);
        assert!(decision.is_approved());
        assert!(!decision.creates_capability());

        let deny = ApprovalDecision::new(Uuid::new_v4(), ApprovalOption::Deny);
        assert!(!deny.is_approved());

        let always = ApprovalDecision::new(Uuid::new_v4(), ApprovalOption::AllowAlways);
        assert!(always.creates_capability());
    }

    #[test]
    fn test_user_input() {
        let input = UserInput::new("Hello, world!");
        assert_eq!(input.content, "Hello, world!");
        assert!(input.attachments.is_empty());
    }

    // -- Blanket adapter delegation tests --

    use crate::identity::FrontendType;
    use crate::input::{ContextIdentifier, MessageId, TaggedMessage};
    use crate::verification::{VerificationRequest, VerificationResponse};

    /// Minimal stub that implements [`Frontend`] for adapter blanket tests.
    struct StubFrontend;

    #[async_trait]
    impl Frontend for StubFrontend {
        fn get_context(&self) -> FrontendContext {
            FrontendContext::new(
                ContextIdentifier::DirectMessage {
                    participant_ids: vec![],
                },
                FrontendUser::new("stub"),
                ChannelInfo::dm("stub-dm"),
                FrontendSessionInfo::new(),
            )
        }

        async fn elicit(&self, request: ElicitationRequest) -> FrontendResult<ElicitationResponse> {
            Ok(ElicitationResponse::cancel(request.request_id))
        }

        async fn elicit_url(
            &self,
            request: UrlElicitationRequest,
        ) -> FrontendResult<UrlElicitationResponse> {
            Ok(UrlElicitationResponse::not_completed(request.request_id))
        }

        async fn request_approval(
            &self,
            request: ApprovalRequest,
        ) -> FrontendResult<ApprovalDecision> {
            Ok(ApprovalDecision::new(
                request.request_id,
                ApprovalOption::AllowOnce,
            ))
        }

        fn show_status(&self, _message: &str) {}
        fn show_error(&self, _error: &str) {}

        async fn receive_input(&self) -> Option<UserInput> {
            None
        }

        async fn resolve_identity(&self, _frontend_user_id: &str) -> Option<crate::AstridUserId> {
            None
        }

        async fn get_message(&self, _message_id: &MessageId) -> Option<TaggedMessage> {
            None
        }

        async fn send_verification(
            &self,
            _user_id: &str,
            _request: VerificationRequest,
        ) -> FrontendResult<VerificationResponse> {
            Err(FrontendError::Internal("stub: not implemented".into()))
        }

        async fn send_link_code(&self, _user_id: &str, _code: &str) -> FrontendResult<()> {
            Ok(())
        }

        fn frontend_type(&self) -> FrontendType {
            FrontendType::Cli
        }
    }

    #[tokio::test]
    async fn blanket_approval_adapter_delegates() {
        let stub = StubFrontend;
        let req = ApprovalRequest::new("test_op", "test approval");
        let result: crate::connector::ConnectorResult<ApprovalDecision> =
            crate::connector::ApprovalAdapter::request_approval(&stub, req).await;
        assert!(result.is_ok());
        let decision = result.unwrap();
        assert_eq!(decision.decision, ApprovalOption::AllowOnce);
    }

    #[tokio::test]
    async fn blanket_elicitation_adapter_delegates() {
        let stub = StubFrontend;
        let req = ElicitationRequest::new("test-server", "need input");
        let result: crate::connector::ConnectorResult<ElicitationResponse> =
            crate::connector::ElicitationAdapter::elicit(&stub, req).await;
        assert!(result.is_ok());
        let resp = result.unwrap();
        assert!(matches!(resp.action, ElicitationAction::Cancel));
    }

    /// Stub that returns errors, for testing the blanket adapter error path.
    struct ErrorStubFrontend;

    #[async_trait]
    impl Frontend for ErrorStubFrontend {
        fn get_context(&self) -> FrontendContext {
            FrontendContext::new(
                ContextIdentifier::DirectMessage {
                    participant_ids: vec![],
                },
                FrontendUser::new("err-stub"),
                ChannelInfo::dm("err-dm"),
                FrontendSessionInfo::new(),
            )
        }

        async fn elicit(
            &self,
            _request: ElicitationRequest,
        ) -> FrontendResult<ElicitationResponse> {
            Err(FrontendError::ElicitationFailed("stub error".into()))
        }

        async fn elicit_url(
            &self,
            request: UrlElicitationRequest,
        ) -> FrontendResult<UrlElicitationResponse> {
            Ok(UrlElicitationResponse::not_completed(request.request_id))
        }

        async fn request_approval(
            &self,
            _request: ApprovalRequest,
        ) -> FrontendResult<ApprovalDecision> {
            Err(FrontendError::ApprovalDenied {
                reason: "stub denied".into(),
            })
        }

        fn show_status(&self, _message: &str) {}
        fn show_error(&self, _error: &str) {}

        async fn receive_input(&self) -> Option<UserInput> {
            None
        }

        async fn resolve_identity(&self, _frontend_user_id: &str) -> Option<crate::AstridUserId> {
            None
        }

        async fn get_message(&self, _message_id: &MessageId) -> Option<TaggedMessage> {
            None
        }

        async fn send_verification(
            &self,
            _user_id: &str,
            _request: VerificationRequest,
        ) -> FrontendResult<VerificationResponse> {
            Err(FrontendError::Internal("not implemented".into()))
        }

        async fn send_link_code(&self, _user_id: &str, _code: &str) -> FrontendResult<()> {
            Ok(())
        }

        fn frontend_type(&self) -> FrontendType {
            FrontendType::Cli
        }
    }

    #[tokio::test]
    async fn blanket_approval_adapter_propagates_security_error() {
        let stub = ErrorStubFrontend;
        let req = ApprovalRequest::new("op", "denied test");
        let result: crate::connector::ConnectorResult<ApprovalDecision> =
            crate::connector::ApprovalAdapter::request_approval(&stub, req).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::connector::ConnectorError::Internal(_)));
    }

    #[tokio::test]
    async fn blanket_elicitation_adapter_propagates_security_error() {
        let stub = ErrorStubFrontend;
        let req = ElicitationRequest::new("srv", "fail test");
        let result: crate::connector::ConnectorResult<ElicitationResponse> =
            crate::connector::ElicitationAdapter::elicit(&stub, req).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::connector::ConnectorError::Internal(_)));
    }
}
