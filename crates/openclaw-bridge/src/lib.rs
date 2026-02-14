//! `OpenClaw` Tool Plugin Compatibility Shim (WS-5).
//!
//! Converts `OpenClaw` tool plugins into Astralis WASM plugins. The ~20% of
//! `OpenClaw` plugins that are pure tool plugins can run inside the Astralis WASM
//! sandbox (Tier 1 path) with full security enforcement.

#![deny(unsafe_code)]

pub mod bundler;
pub mod compiler;
pub mod error;
pub mod manifest;
pub mod output;
pub mod shim;
