//! Integration test crate for Astrid.
//!
//! This crate exists solely for integration testing. It is `publish = false`
//! and has no library code — all tests live in `tests/`.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]
