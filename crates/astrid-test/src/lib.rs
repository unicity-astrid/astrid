//! Astrid Test - Shared test utilities for the Astrid runtime.
//!
//! This crate provides mock implementations and test helpers that can be
//! used across multiple Astrid crates as a dev-dependency.
//!
//! # Usage
//!
//! Add to your crate's `Cargo.toml`:
//!
//! ```toml
//! [dev-dependencies]
//! astrid-test.workspace = true
//! ```
//!
//! Then use in your tests:
//!
//! ```rust,ignore
//! #[cfg(test)]
//! mod tests {
//!     use astrid_test::{MockFrontend, test_approval_request};
//!     use astrid_core::ApprovalOption;
//!
//!     #[tokio::test]
//!     async fn test_approval_flow() {
//!         let frontend = MockFrontend::new()
//!             .with_approval_response(ApprovalOption::AllowOnce);
//!
//!         let request = test_approval_request();
//!         let decision = frontend.request_approval(request).await.unwrap();
//!
//!         assert!(decision.is_approved());
//!     }
//! }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

pub mod fixtures;
pub mod harness;
pub mod mock_llm;
pub mod mocks;

pub use fixtures::*;
pub use harness::*;
pub use mock_llm::*;
pub use mocks::*;
