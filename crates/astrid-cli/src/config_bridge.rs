//! Bridge from `astrid_config::Config` to domain types.
//!
//! Re-exports the canonical bridge from `astrid_runtime::config_bridge`.
//! This module exists for backwards compatibility — all conversion logic
//! lives in `astrid-runtime` so it can be shared between the CLI and the
//! gateway daemon.

pub use astrid_runtime::config_bridge::*;
