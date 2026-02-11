//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astralis_test::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,ignore
//! #[cfg(test)]
//! mod tests {
//!     use astralis_test::prelude::*;
//!     use astralis_core::ApprovalOption;
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

// Re-export all public items from the crate root
// The crate already uses glob re-exports, so we mirror that pattern
pub use crate::fixtures::*;
pub use crate::harness::*;
pub use crate::mock_llm::*;
pub use crate::mocks::*;
