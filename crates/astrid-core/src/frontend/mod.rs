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


pub mod error;
pub mod traits;
pub mod types;

pub use error::{FrontendError, FrontendResult};
pub use traits::{ArcFrontend, Frontend};
pub use types::*;
