//! Consciousness bridge library — exposes modules for integration tests.
//!
//! The binary is the primary artifact; this lib target exists so integration
//! tests can import internal types. Pedantic doc lints are relaxed since
//! these are not public API.
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod agency;
pub mod audio;
pub mod autonomous;
pub mod autoresearch;
pub mod chimera;
pub mod chimera_prime;
pub mod codec;
pub mod codec_explorer;
pub mod codec_phase_space;
pub mod codec_scored_surface;
pub mod condition_metrics;
pub mod db;
pub mod journal;
pub mod llm;
#[path = "../../shared/managed_dir.rs"]
pub mod managed_dir;
pub mod mcp;
pub mod memory;
pub mod paths;
pub mod prompt_budget;
pub mod reflective;
pub mod self_model;
pub mod spectral_viz;
pub mod types;
pub mod ws;

#[cfg(test)]
mod llm_tests;
