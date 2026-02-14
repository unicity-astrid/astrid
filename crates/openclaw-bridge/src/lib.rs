//! `OpenClaw` Tool Plugin Compatibility Shim (WS-5).
//!
//! Converts `OpenClaw` tool plugins into Astralis WASM plugins. The ~20% of
//! `OpenClaw` plugins that are pure tool plugins can run inside the Astralis WASM
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

pub mod compiler;
pub mod error;
pub mod export_stitch;
pub mod manifest;
pub mod output;
pub mod shim;
pub mod transpiler;
