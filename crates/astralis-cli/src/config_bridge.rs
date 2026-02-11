//! Bridge from `astralis_config::Config` to domain types.
//!
//! Re-exports the canonical bridge from `astralis_runtime::config_bridge`.
//! This module exists for backwards compatibility — all conversion logic
//! lives in `astralis-runtime` so it can be shared between the CLI and the
//! gateway daemon.

pub use astralis_runtime::config_bridge::*;
