//! Elicitation capability handler traits.
//!
//! These traits use canonical elicitation types from `astrid-core` (single
//! source of truth). No MCP-local duplicates exist.

use async_trait::async_trait;

// Canonical elicitation types from astrid-core (single source of truth).
use astrid_core::{
    ElicitationRequest, ElicitationResponse, UrlElicitationRequest, UrlElicitationResponse,
};

/// Handler for server requests for user input.
///
/// Implementations receive canonical [`ElicitationRequest`] from `astrid-core`
/// and should return an [`ElicitationResponse`] after collecting user input.
#[async_trait]
pub trait ElicitationHandler: Send + Sync {
    /// Handle an elicitation request from a server.
    ///
    /// The implementation should:
    /// 1. Display the message to the user
    /// 2. Collect their response based on the schema
    /// 3. Return the appropriate action (submit, cancel, dismiss)
    async fn handle_elicitation(&self, request: ElicitationRequest) -> ElicitationResponse;
}

/// Handler for URL-based elicitation (OAuth, payments).
///
/// Implementations receive canonical [`UrlElicitationRequest`] from `astrid-core`
/// and should return a [`UrlElicitationResponse`] after the user completes the flow.
#[async_trait]
pub trait UrlElicitationHandler: Send + Sync {
    /// Handle a URL elicitation request from a server.
    ///
    /// The implementation should:
    /// 1. Open the URL in the user's browser
    /// 2. Listen for a callback (if OAuth/payment)
    /// 3. Return the result
    ///
    /// IMPORTANT: For payment flows, the LLM should NEVER see the amounts.
    /// The client handles the payment UI directly.
    async fn handle_url_elicitation(
        &self,
        request: UrlElicitationRequest,
    ) -> UrlElicitationResponse;
}
