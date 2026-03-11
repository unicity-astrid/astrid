//! Frontend types - approval, elicitation, and user interaction primitives.
//!
//! These types are the shared vocabulary between the kernel, capsules, and
//! CLI for user-facing interactions. The old monolithic `Frontend` trait has
//! been removed - frontends are now capsule uplinks (e.g. `astrid-capsule-cli`).
//!
//! # Key Types
//!
//! - [`FrontendContext`] - Current interaction context
//! - [`ApprovalRequest`] / [`ApprovalDecision`] - Approval flow
//! - [`ElicitationRequest`] / [`ElicitationResponse`] - MCP elicitation

/// Core types for frontends.
pub(crate) mod types;

pub use types::*;
