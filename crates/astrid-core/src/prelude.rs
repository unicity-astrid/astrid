//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_core::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust
//! use astrid_core::prelude::*;
//!
//! // Now you have access to:
//! // - SecurityError, SecurityResult
//! // - Uplink types
//! // - Identity types
//! // - Common types like SessionId, Permission, RiskLevel
//! ```

// Errors
pub use crate::{SecurityError, SecurityResult};

// Frontend types (approval, elicitation, etc.)
pub use crate::{
    ApprovalDecision, ApprovalOption, ApprovalRequest, ElicitationAction, ElicitationRequest,
    ElicitationResponse, ElicitationSchema, FrontendContext, FrontendSessionInfo, FrontendUser,
    SelectOption, UrlElicitationRequest, UrlElicitationResponse, UrlElicitationType, UserInput,
};

// Identity
pub use crate::{AstridUserId, FrontendLink, FrontendType};

// Input classification
pub use crate::{ContextIdentifier, MessageId, TaggedMessage};

// Common types
pub use crate::{AgentId, Permission, RiskLevel, SessionId, Timestamp, TokenId};

// Retry utilities
pub use crate::RetryConfig;

// Uplink
pub use crate::{
    InboundMessage, InboundMessageBuilder, OutboundMessage, OutboundMessageBuilder,
    UplinkCapabilities, UplinkDescriptor, UplinkDescriptorBuilder, UplinkError, UplinkId,
    UplinkProfile, UplinkResult, UplinkSource,
};
