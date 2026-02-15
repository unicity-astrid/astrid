//! Mock implementations for testing.

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use astrid_core::{
    ApprovalDecision, ApprovalOption, ApprovalRequest, AstridUserId, ElicitationRequest,
    ElicitationResponse, Frontend, FrontendContext, FrontendSessionInfo, FrontendType,
    FrontendUser, MessageId, SecurityResult, TaggedMessage, UrlElicitationRequest,
    UrlElicitationResponse, UserInput, VerificationRequest, VerificationResponse,
    frontend::{ChannelInfo, ChannelType},
    input::ContextIdentifier,
};

/// Mock implementation of the `Frontend` trait for testing.
///
/// Uses `std::sync::Mutex` internally to allow both sync and async usage
/// without requiring a tokio runtime for builder methods.
#[derive(Debug, Clone)]
pub struct MockFrontend {
    /// Queued elicitation responses.
    elicitation_responses: Arc<Mutex<VecDeque<ElicitationResponse>>>,
    /// Queued approval decisions.
    approval_responses: Arc<Mutex<VecDeque<ApprovalOption>>>,
    /// Queued user inputs.
    user_inputs: Arc<Mutex<VecDeque<UserInput>>>,
    /// Captured status messages.
    status_messages: Arc<Mutex<Vec<String>>>,
    /// Captured error messages.
    error_messages: Arc<Mutex<Vec<String>>>,
    /// Default approval option when queue is empty.
    default_approval: ApprovalOption,
    /// Context to return.
    context: FrontendContext,
}

impl MockFrontend {
    /// Create a new mock frontend.
    #[must_use]
    pub fn new() -> Self {
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        Self {
            elicitation_responses: Arc::new(Mutex::new(VecDeque::new())),
            approval_responses: Arc::new(Mutex::new(VecDeque::new())),
            user_inputs: Arc::new(Mutex::new(VecDeque::new())),
            status_messages: Arc::new(Mutex::new(Vec::new())),
            error_messages: Arc::new(Mutex::new(Vec::new())),
            default_approval: ApprovalOption::Deny,
            context: FrontendContext::new(
                ContextIdentifier::CliSession {
                    session_id: session_id.to_string(),
                    user_id,
                },
                FrontendUser::new("test-user").with_astrid_id(user_id),
                ChannelInfo {
                    id: "test-channel".to_string(),
                    name: Some("Test Channel".to_string()),
                    channel_type: ChannelType::Cli,
                    guild_id: None,
                },
                FrontendSessionInfo::new(),
            ),
        }
    }

    /// Queue an elicitation response.
    ///
    /// This works in both sync and async contexts without blocking.
    #[must_use]
    pub fn with_elicitation_response(self, response: ElicitationResponse) -> Self {
        if let Ok(mut guard) = self.elicitation_responses.lock() {
            guard.push_back(response);
        }
        self
    }

    /// Queue an approval response.
    ///
    /// This works in both sync and async contexts without blocking.
    #[must_use]
    pub fn with_approval_response(self, option: ApprovalOption) -> Self {
        if let Ok(mut guard) = self.approval_responses.lock() {
            guard.push_back(option);
        }
        self
    }

    /// Set the default approval option (used when queue is empty).
    #[must_use]
    pub fn with_default_approval(mut self, option: ApprovalOption) -> Self {
        self.default_approval = option;
        self
    }

    /// Queue a user input.
    ///
    /// This works in both sync and async contexts without blocking.
    #[must_use]
    pub fn with_user_input(self, input: UserInput) -> Self {
        if let Ok(mut guard) = self.user_inputs.lock() {
            guard.push_back(input);
        }
        self
    }

    /// Queue an approval response.
    pub fn queue_approval(&self, option: ApprovalOption) {
        if let Ok(mut guard) = self.approval_responses.lock() {
            guard.push_back(option);
        }
    }

    /// Queue a user input.
    pub fn queue_input(&self, input: UserInput) {
        if let Ok(mut guard) = self.user_inputs.lock() {
            guard.push_back(input);
        }
    }

    /// Get captured status messages.
    #[must_use]
    pub fn get_status_messages(&self) -> Vec<String> {
        self.status_messages
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Get captured error messages.
    #[must_use]
    pub fn get_error_messages(&self) -> Vec<String> {
        self.error_messages
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Clear all captured messages.
    pub fn clear_messages(&self) {
        if let Ok(mut guard) = self.status_messages.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.error_messages.lock() {
            guard.clear();
        }
    }
}

impl Default for MockFrontend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Frontend for MockFrontend {
    fn get_context(&self) -> FrontendContext {
        self.context.clone()
    }

    async fn elicit(&self, request: ElicitationRequest) -> SecurityResult<ElicitationResponse> {
        let response = self
            .elicitation_responses
            .lock()
            .ok()
            .and_then(|mut g| g.pop_front());
        if let Some(response) = response {
            Ok(response)
        } else {
            // Default: cancel
            Ok(ElicitationResponse::cancel(request.request_id))
        }
    }

    async fn elicit_url(
        &self,
        request: UrlElicitationRequest,
    ) -> SecurityResult<UrlElicitationResponse> {
        Ok(UrlElicitationResponse::completed(request.request_id))
    }

    async fn request_approval(&self, request: ApprovalRequest) -> SecurityResult<ApprovalDecision> {
        let option = self
            .approval_responses
            .lock()
            .ok()
            .and_then(|mut g| g.pop_front())
            .unwrap_or(self.default_approval);
        Ok(ApprovalDecision::new(request.request_id, option))
    }

    fn show_status(&self, message: &str) {
        if let Ok(mut guard) = self.status_messages.lock() {
            guard.push(message.to_string());
        }
    }

    fn show_error(&self, error: &str) {
        if let Ok(mut guard) = self.error_messages.lock() {
            guard.push(error.to_string());
        }
    }

    async fn receive_input(&self) -> Option<UserInput> {
        self.user_inputs.lock().ok().and_then(|mut g| g.pop_front())
    }

    async fn resolve_identity(&self, _frontend_user_id: &str) -> Option<AstridUserId> {
        Some(AstridUserId::new())
    }

    async fn get_message(&self, _message_id: &MessageId) -> Option<TaggedMessage> {
        None
    }

    async fn send_verification(
        &self,
        _user_id: &str,
        request: VerificationRequest,
    ) -> SecurityResult<VerificationResponse> {
        Ok(VerificationResponse::confirmed(request.request_id))
    }

    async fn send_link_code(&self, _user_id: &str, _code: &str) -> SecurityResult<()> {
        Ok(())
    }

    fn frontend_type(&self) -> FrontendType {
        FrontendType::Cli
    }
}

/// Mock event bus for capturing emitted events.
///
/// Uses `std::sync::Mutex` for simplicity and sync/async compatibility.
#[derive(Debug, Clone, Default)]
pub struct MockEventBus {
    /// Captured events.
    events: Arc<Mutex<Vec<MockEvent>>>,
}

/// A captured event.
#[derive(Debug, Clone)]
pub struct MockEvent {
    /// Event type/name.
    pub event_type: String,
    /// Event payload as JSON.
    pub payload: serde_json::Value,
}

impl MockEventBus {
    /// Create a new mock event bus.
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Emit an event.
    pub fn emit(&self, event_type: impl Into<String>, payload: serde_json::Value) {
        if let Ok(mut guard) = self.events.lock() {
            guard.push(MockEvent {
                event_type: event_type.into(),
                payload,
            });
        }
    }

    /// Get all captured events.
    #[must_use]
    pub fn get_events(&self) -> Vec<MockEvent> {
        self.events.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Get events of a specific type.
    #[must_use]
    pub fn get_events_of_type(&self, event_type: &str) -> Vec<MockEvent> {
        self.events
            .lock()
            .map(|g| {
                g.iter()
                    .filter(|e| e.event_type == event_type)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Clear all captured events.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.events.lock() {
            guard.clear();
        }
    }

    /// Check if any event of the given type was emitted.
    #[must_use]
    pub fn has_event(&self, event_type: &str) -> bool {
        self.events
            .lock()
            .map(|g| g.iter().any(|e| e.event_type == event_type))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_frontend_approval() {
        let frontend = MockFrontend::new();
        frontend.queue_approval(ApprovalOption::AllowOnce);

        let request = ApprovalRequest::new("test_op", "Test operation");
        let decision = frontend.request_approval(request).await.unwrap();

        assert!(decision.is_approved());
        assert_eq!(decision.decision, ApprovalOption::AllowOnce);
    }

    #[tokio::test]
    async fn test_mock_frontend_default_denial() {
        let frontend = MockFrontend::new();

        let request = ApprovalRequest::new("test_op", "Test operation");
        let decision = frontend.request_approval(request).await.unwrap();

        assert!(!decision.is_approved());
    }

    #[tokio::test]
    async fn test_mock_frontend_messages() {
        let frontend = MockFrontend::new();

        frontend.show_status("Status 1");
        frontend.show_status("Status 2");
        frontend.show_error("Error 1");

        let statuses = frontend.get_status_messages();
        let errors = frontend.get_error_messages();

        assert_eq!(statuses.len(), 2);
        assert_eq!(errors.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_event_bus() {
        let bus = MockEventBus::new();

        bus.emit("test_event", serde_json::json!({"key": "value"}));
        bus.emit("other_event", serde_json::json!({}));

        assert!(bus.has_event("test_event"));
        assert!(!bus.has_event("nonexistent"));

        let test_events = bus.get_events_of_type("test_event");
        assert_eq!(test_events.len(), 1);
    }
}
