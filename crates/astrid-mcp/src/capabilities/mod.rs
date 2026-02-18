//! MCP client capabilities handlers.
//!
//! These handlers implement client-side capabilities from the MCP Nov 2025 spec:
//! - Sampling: Server-initiated LLM calls
//! - Roots: Server inquiries about operational boundaries
//! - Elicitation: Server requests for user input (canonical types from `astrid-core`)
//! - URL Elicitation: OAuth flows, payments, credentials (canonical types from `astrid-core`)

mod client;
mod convert;
mod elicitation;
mod handler;
mod roots;
mod sampling;

pub use client::{
    AstridClientHandler, BridgeChannelCapabilities, BridgeChannelDefinition, BridgeChannelInfo,
    ServerNotice,
};
pub use elicitation::{ElicitationHandler, UrlElicitationHandler};
pub use handler::CapabilitiesHandler;
pub use roots::{Root, RootsHandler, RootsRequest, RootsResponse};
pub use sampling::{
    SamplingContent, SamplingHandler, SamplingMessage, SamplingRequest, SamplingResponse,
};
