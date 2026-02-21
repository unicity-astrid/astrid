//! Integration test crate for Astrid.
//!
//! This crate exists solely for integration testing. It is `publish = false`
//! and has no library code — all tests live in `tests/`.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]
