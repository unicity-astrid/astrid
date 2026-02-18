//! Astrid Core - Foundation types and traits for the Astrid secure agent runtime.
//!
//! This crate provides:
//! - Error types for security operations
//! - Input classification and message attribution
//! - Identity management across frontends
//! - The `Frontend` trait for different UI implementations
//! - Common types used throughout the runtime
//! - Version management for state migrations
//! - Retry utilities with exponential backoff

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

pub mod dirs;
pub mod env_policy;
pub mod error;
pub mod frontend;
pub mod hook_event;
pub mod identity;
pub mod input;
pub mod plugin_abi;
pub mod retry;
pub mod types;
pub mod utils;
pub mod verification;
pub mod version;

pub mod connector;

pub use error::{SecurityError, SecurityResult};
pub use frontend::{
    ApprovalDecision, ApprovalOption, ApprovalRequest, ElicitationAction, ElicitationRequest,
    ElicitationResponse, ElicitationSchema, Frontend, FrontendContext, FrontendSessionInfo,
    FrontendUser, SelectOption, UrlElicitationRequest, UrlElicitationResponse, UrlElicitationType,
    UserInput,
};
pub use hook_event::HookEvent;
pub use identity::{AstridUserId, FrontendLink, FrontendType, LinkVerificationMethod};
pub use input::{ContextIdentifier, MessageId, TaggedMessage};
pub use retry::{RetryConfig, RetryOutcome, retry};
pub use types::{AgentId, Permission, RiskLevel, SessionId, Timestamp, TokenId};
pub use utils::truncate_to_boundary;
pub use verification::{VerificationRequest, VerificationResponse};
pub use version::{Version, VersionParseError, Versioned};

// Connector types
pub use connector::{
    ApprovalAdapter, ConnectorCapabilities, ConnectorDescriptor, ConnectorDescriptorBuilder,
    ConnectorError, ConnectorId, ConnectorProfile, ConnectorResult, ConnectorSource,
    ElicitationAdapter, InboundAdapter, InboundMessage, InboundMessageBuilder, OutboundAdapter,
    OutboundMessage, OutboundMessageBuilder,
};
