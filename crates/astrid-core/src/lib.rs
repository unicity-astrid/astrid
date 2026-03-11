//! Astrid Core - Foundation types and traits for the Astrid secure agent runtime.
//!
//! This crate provides:
//! - Error types for security operations
//! - Input classification and message attribution
//! - Identity management across frontends
//! - Uplink types for capsule integration
//! - Common types used throughout the runtime
//! - Retry utilities with exponential backoff

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod prelude;

pub mod dirs;
pub mod env_policy;
pub(crate) mod error;
pub mod frontend;
pub mod hook_event;
pub mod identity;
pub(crate) mod input;
pub mod plugin_abi;
pub mod retry;
pub mod types;
pub(crate) mod utils;
pub(crate) mod version;

pub mod uplink;

/// HTTP utilities and protections (e.g. SSRF prevention)
#[cfg(feature = "reqwest")]
pub mod http;

pub use frontend::{
    ApprovalDecision, ApprovalOption, ApprovalRequest, ElicitationAction, ElicitationRequest,
    ElicitationResponse, ElicitationSchema, FrontendContext, FrontendUser, SelectOption,
    UrlElicitationRequest, UrlElicitationResponse, UrlElicitationType, UserInput,
};
pub use hook_event::HookEvent;
pub use identity::{AstridUserId, FrontendType};
pub use input::MessageId;
pub use retry::RetryConfig;
pub use types::{AgentId, Permission, RiskLevel, SessionId, Timestamp, TokenId};
pub use utils::truncate_to_boundary;

// Uplink types
pub use uplink::{
    InboundMessage, InboundMessageBuilder, MAX_UPLINKS_PER_PLUGIN, OutboundMessage,
    OutboundMessageBuilder, UplinkCapabilities, UplinkDescriptor, UplinkDescriptorBuilder,
    UplinkError, UplinkId, UplinkProfile, UplinkResult, UplinkSource,
};
