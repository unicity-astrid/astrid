//! Daemon-side Frontend implementation.
//!
//! Bridges the `Frontend` trait to IPC: when the runtime calls `request_approval()`,
//! the `DaemonFrontend` serializes the request, pushes it as a `DaemonEvent` to the
//! connected CLI client via the subscription channel, and waits for the CLI to respond
//! via an RPC call.

use std::collections::HashMap;
use std::sync::Arc;

use astralis_core::frontend::ChannelInfo;
use astralis_core::identity::AstralisUserId;
use astralis_core::input::{ContextIdentifier, MessageId, TaggedMessage};
use astralis_core::verification::{VerificationRequest, VerificationResponse};
use astralis_core::{
    ApprovalDecision, ApprovalRequest, ElicitationRequest, ElicitationResponse, Frontend,
    FrontendContext, FrontendSessionInfo, FrontendType, FrontendUser, SecurityError,
    SecurityResult, UrlElicitationRequest, UrlElicitationResponse, UserInput,
};
use async_trait::async_trait;
use tokio::sync::{Mutex, broadcast, oneshot};
use tracing::{debug, warn};

use crate::rpc::DaemonEvent;

/// A pending request that's waiting for a CLI response.
struct PendingRequest<T> {
    tx: oneshot::Sender<T>,
}

/// Daemon-side Frontend implementation.
///
/// Bridges the `Frontend` trait to IPC channels:
/// - Outgoing events (approval/elicitation requests, status) → `broadcast::Sender<DaemonEvent>`
/// - Incoming responses (approval/elicitation responses) → registered oneshot channels
pub struct DaemonFrontend {
    /// Channel to send events to connected CLI clients.
    event_tx: broadcast::Sender<DaemonEvent>,
    /// Pending approval requests waiting for CLI responses.
    pending_approvals: Arc<Mutex<HashMap<String, PendingRequest<ApprovalDecision>>>>,
    /// Pending elicitation requests waiting for CLI responses.
    pending_elicitations: Arc<Mutex<HashMap<String, PendingRequest<ElicitationResponse>>>>,
    /// Session info for context.
    session_info: FrontendSessionInfo,
}

impl DaemonFrontend {
    /// Create a new daemon frontend.
    #[must_use]
    pub fn new(event_tx: broadcast::Sender<DaemonEvent>) -> Self {
        Self {
            event_tx,
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            pending_elicitations: Arc::new(Mutex::new(HashMap::new())),
            session_info: FrontendSessionInfo::new(),
        }
    }

    /// Resolve a pending approval request with the CLI's decision.
    ///
    /// Called by the RPC server when it receives an `approval_response` call from the CLI.
    pub async fn resolve_approval(&self, request_id: &str, decision: ApprovalDecision) -> bool {
        let mut pending = self.pending_approvals.lock().await;
        if let Some(req) = pending.remove(request_id) {
            let _ = req.tx.send(decision);
            true
        } else {
            warn!(request_id, "No pending approval found for request");
            false
        }
    }

    /// Resolve a pending elicitation request with the CLI's response.
    ///
    /// Called by the RPC server when it receives an `elicitation_response` call from the CLI.
    pub async fn resolve_elicitation(
        &self,
        request_id: &str,
        response: ElicitationResponse,
    ) -> bool {
        let mut pending = self.pending_elicitations.lock().await;
        if let Some(req) = pending.remove(request_id) {
            let _ = req.tx.send(response);
            true
        } else {
            warn!(request_id, "No pending elicitation found for request");
            false
        }
    }

    /// Send an event to connected clients, ignoring errors (no subscribers).
    fn send_event(&self, event: DaemonEvent) {
        // If no subscribers, the event is dropped (expected when no CLI is connected).
        let _ = self.event_tx.send(event);
    }
}

#[async_trait]
impl Frontend for DaemonFrontend {
    fn get_context(&self) -> FrontendContext {
        FrontendContext::new(
            ContextIdentifier::cli_session("daemon".to_string(), uuid::Uuid::nil()),
            FrontendUser::new("daemon_user"),
            ChannelInfo {
                id: "daemon".to_string(),
                name: Some("Daemon".to_string()),
                channel_type: astralis_core::frontend::ChannelType::Cli,
                guild_id: None,
            },
            self.session_info.clone(),
        )
    }

    async fn elicit(&self, request: ElicitationRequest) -> SecurityResult<ElicitationResponse> {
        let request_id = request.request_id.to_string();

        // Create a oneshot channel to receive the CLI's response.
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_elicitations.lock().await;
            pending.insert(request_id.clone(), PendingRequest { tx });
        }

        // Send the elicitation request as a DaemonEvent.
        self.send_event(DaemonEvent::ElicitationNeeded {
            request_id: request_id.clone(),
            request,
        });

        debug!(request_id, "Waiting for elicitation response from CLI");

        // Wait for the CLI to respond.
        rx.await.map_err(|_| {
            SecurityError::McpElicitationFailed("CLI disconnected before responding".to_string())
        })
    }

    async fn elicit_url(
        &self,
        request: UrlElicitationRequest,
    ) -> SecurityResult<UrlElicitationResponse> {
        // URL elicitation is not yet supported over IPC.
        // For now, return a "not completed" response.
        warn!(
            url = %request.url,
            "URL elicitation not yet supported over daemon IPC"
        );
        Ok(UrlElicitationResponse::not_completed(request.request_id))
    }

    async fn request_approval(&self, request: ApprovalRequest) -> SecurityResult<ApprovalDecision> {
        let request_id = request.request_id.to_string();

        // Create a oneshot channel to receive the CLI's decision.
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_approvals.lock().await;
            pending.insert(request_id.clone(), PendingRequest { tx });
        }

        // Send the approval request as a DaemonEvent.
        self.send_event(DaemonEvent::ApprovalNeeded {
            request_id: request_id.clone(),
            request,
        });

        debug!(request_id, "Waiting for approval response from CLI");

        // Wait for the CLI to respond.
        rx.await.map_err(|_| SecurityError::ApprovalDenied {
            reason: "CLI disconnected before responding".to_string(),
        })
    }

    fn show_status(&self, message: &str) {
        self.send_event(DaemonEvent::Text(message.to_string()));
    }

    fn show_error(&self, error: &str) {
        self.send_event(DaemonEvent::Error(error.to_string()));
    }

    fn tool_started(&self, id: &str, name: &str, args: &serde_json::Value) {
        self.send_event(DaemonEvent::ToolCallStart {
            id: id.to_string(),
            name: name.to_string(),
            args: args.clone(),
        });
    }

    fn tool_completed(&self, id: &str, result: &str, is_error: bool) {
        self.send_event(DaemonEvent::ToolCallResult {
            id: id.to_string(),
            result: result.to_string(),
            is_error,
        });
    }

    async fn receive_input(&self) -> Option<UserInput> {
        // Input comes via RPC `send_input`, not through this method.
        // This method should never be called on the daemon frontend.
        None
    }

    async fn resolve_identity(&self, _frontend_user_id: &str) -> Option<AstralisUserId> {
        None
    }

    async fn get_message(&self, _message_id: &MessageId) -> Option<TaggedMessage> {
        None
    }

    async fn send_verification(
        &self,
        _user_id: &str,
        _request: VerificationRequest,
    ) -> SecurityResult<VerificationResponse> {
        Err(SecurityError::IdentityVerificationFailed(
            "Verification not supported over daemon IPC".to_string(),
        ))
    }

    async fn send_link_code(&self, _user_id: &str, _code: &str) -> SecurityResult<()> {
        Ok(())
    }

    fn frontend_type(&self) -> FrontendType {
        FrontendType::Cli
    }
}
