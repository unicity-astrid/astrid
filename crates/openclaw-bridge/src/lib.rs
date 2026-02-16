//! `OpenClaw` Tool Plugin Compatibility Shim (WS-5).
//!
//! Converts `OpenClaw` tool plugins into Astrid WASM plugins. The ~20% of
//! `OpenClaw` plugins that are pure tool plugins can run inside the Astrid WASM
//! sandbox (Tier 1 path) with full security enforcement.
//!
//! ## Compilation Pipeline
//!
//! ```text
//! Plugin.ts → [OXC transpiler] → Plugin.js → [shim.rs] → shimmed.js
//!   → [Wizer + QuickJS kernel] → raw.wasm → [export stitcher] → plugin.wasm
//! ```
//!
//! All stages are pure Rust — no external tools required.

#![deny(unsafe_code)]

/// Bridge crate version, used for compilation cache invalidation.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod cache;
pub mod compiler;
pub mod error;
pub mod export_stitch;
pub mod manifest;
pub mod node_bridge;
pub mod output;
pub mod shim;
pub mod tier;
pub mod transpiler;
