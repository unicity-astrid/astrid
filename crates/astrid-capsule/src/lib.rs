//! Core runtime management for User-Space Capsules in Astrid OS.
//!
//! This crate succeeds `astrid-plugins` and implements the Phase 4
//! "Manifest-First" architecture. It provides the definition for `Capsule.toml`
//! manifests, handles discovery, and routes execution to the appropriate
//! environments (WASM sandboxes, legacy host processes, or OpenClaw bridges).

pub mod capsule;
pub mod context;
pub mod discovery;
pub mod engine;
pub mod error;
pub mod loader;
pub mod manifest;
pub mod registry;
pub mod security;
pub mod tool;
pub mod watcher;
