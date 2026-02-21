use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::frontend::{ApprovalDecision, ApprovalRequest, ElicitationRequest, ElicitationResponse};
use super::types::{InboundMessage, OutboundMessage};
use super::error::ConnectorResult;

// Adapter traits
// ---------------------------------------------------------------------------

/// Produces inbound messages from an external source.
///
/// Call [`subscribe`](Self::subscribe) to obtain a channel receiver that
/// yields [`InboundMessage`]s as they arrive.
///
/// # Single-subscriber semantics
///
/// This is a **single-subscriber** adapter. The first call to `subscribe`
/// creates the internal channel and returns the [`mpsc::Receiver`]. Subsequent
/// calls should return [`ConnectorError::UnsupportedOperation`] — the adapter
/// holds the `Sender` half internally. If the `Receiver` is dropped, inflight
/// sends will fail and the adapter may treat the subscriber as disconnected.
#[async_trait]
pub trait InboundAdapter: Send + Sync {
    /// Subscribe to inbound messages.
    ///
    /// Returns the receive half of an internal `mpsc` channel. May only be
    /// called once; subsequent calls should fail with
    /// [`ConnectorError::UnsupportedOperation`].
    async fn subscribe(&self) -> ConnectorResult<mpsc::Receiver<InboundMessage>>;
}

/// Sends outbound messages to an external destination.
#[async_trait]
pub trait OutboundAdapter: Send + Sync {
    /// Send a message through this connector.
    async fn send(&self, message: OutboundMessage) -> ConnectorResult<()>;
}

/// Presents approval requests to a human decision-maker.
#[async_trait]
pub trait ApprovalAdapter: Send + Sync {
    /// Request human approval for an operation.
    async fn request_approval(&self, request: ApprovalRequest)
    -> ConnectorResult<ApprovalDecision>;
}

/// Presents elicitation requests to a human for structured input.
#[async_trait]
pub trait ElicitationAdapter: Send + Sync {
    /// Elicit structured input from a human.
    async fn elicit(&self, request: ElicitationRequest) -> ConnectorResult<ElicitationResponse>;
}

// ---------------------------------------------------------------------------
