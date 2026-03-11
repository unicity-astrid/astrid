//! Astrid Test - Shared test utilities for the Astrid runtime.
//!
//! This crate provides mock implementations, fixtures, and test helpers that
//! can be used across multiple Astrid crates as a dev-dependency.
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
//!     use astrid_test::prelude::*;
//!
//!     #[test]
//!     fn test_approval_fixture() {
//!         let req = test_approval_request();
//!         assert_eq!(req.operation, "test_operation");
//!     }
//! }
//! ```

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod prelude;

pub mod fixtures;
pub mod harness;

pub mod mocks;

pub use fixtures::*;
pub use harness::*;

pub use mocks::*;
