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
//! // - Uplink types
//! // - Identity types
//! // - Common types like SessionId, Permission
//! ```

// Elicitation (MCP server-initiated user input)
pub use crate::{
    ElicitationAction, ElicitationRequest, ElicitationResponse, ElicitationSchema, SelectOption,
    UrlElicitationRequest, UrlElicitationResponse, UrlElicitationType,
};

// Common types (approval, permissions, IDs)
pub use crate::{
    AgentId, ApprovalDecision, ApprovalOption, ApprovalRequest, Permission, SessionId, Timestamp,
    TokenId,
};

// Retry utilities
pub use crate::RetryConfig;

// Uplink
pub use crate::{
    InboundMessage, UplinkCapabilities, UplinkDescriptor, UplinkError, UplinkId, UplinkProfile,
    UplinkResult, UplinkSource,
};
