//! Astrid Core - Foundation types for the Astrid secure agent runtime.
//!
//! This crate provides:
//! - Identity management across platforms
//! - Uplink types for capsule integration
//! - Approval and elicitation primitives
//! - Capsule ABI types (WASM host-guest interface)
//! - Common types used throughout the runtime
//! - Retry configuration with exponential backoff

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod prelude;

pub mod capsule_abi;
pub mod dirs;
pub mod elicitation;
pub mod env_policy;
pub mod identity;
pub mod retry;
pub mod types;
pub mod uplink;
pub(crate) mod utils;

pub use elicitation::{
    ElicitationAction, ElicitationRequest, ElicitationResponse, ElicitationSchema, SelectOption,
    UrlElicitationRequest, UrlElicitationResponse, UrlElicitationType,
};
pub use retry::RetryConfig;
pub use types::{
    AgentId, ApprovalDecision, ApprovalOption, ApprovalRequest, Permission, RiskLevel, SessionId,
    Timestamp, TokenId,
};
pub use utils::truncate_to_boundary;

// Uplink types
pub use uplink::{
    InboundMessage, MAX_UPLINKS_PER_CAPSULE, UplinkCapabilities, UplinkDescriptor, UplinkError,
    UplinkId, UplinkProfile, UplinkResult, UplinkSource,
};
