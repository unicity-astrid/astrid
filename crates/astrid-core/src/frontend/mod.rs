//! Frontend Trait - Interface for UI implementations
//!
//! All frontends (CLI, Discord, Web, etc.) implement this trait to provide
//! a consistent interface for user interaction, elicitation, and verification.
//!
//! # Key Types
//!
//! - [`Frontend`] - The main trait all frontends implement
//! - [`FrontendContext`] - Current interaction context
//! - [`ApprovalRequest`] / [`ApprovalDecision`] - Approval flow
//! - [`ElicitationRequest`] / [`ElicitationResponse`] - MCP elicitation
//!
//! # Example Implementation
//!
//! ```rust,ignore
//! use astrid_core::frontend::{Frontend, FrontendContext};
//!
//! struct MyFrontend;
//!
//! #[async_trait::async_trait]
//! impl Frontend for MyFrontend {
//!     fn get_context(&self) -> FrontendContext { ... }
//!     // ... other methods
//! }
//! ```

/// Error types for frontends.
pub(crate) mod error;
/// Trait definitions for frontends.
pub(crate) mod traits;
/// Core types for frontends.
pub(crate) mod types;

pub use error::{FrontendError, FrontendResult};
pub use traits::Frontend;
pub use types::*;
