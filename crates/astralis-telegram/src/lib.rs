//! Astralis Telegram Bot — a thin client for the Astralis agent runtime.
//!
//! Connects to a running `astralisd` daemon via `WebSocket` JSON-RPC and
//! exposes the agent through a Telegram bot interface.
//!
//! This crate can be used as a library (embedded in the daemon) or as a
//! standalone binary (`astralis-telegram`).

// These are pre-existing pedantic lints on items that became public API when
// the crate was split into lib + bin. Suppress at the crate level to avoid
// bloating the diff — they can be addressed in a dedicated cleanup pass.
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]

pub mod approval;
pub mod bot;
pub mod client;
pub mod config;
pub mod elicitation;
pub mod error;
pub mod event_loop;
pub mod format;
pub mod handler;
pub mod session;
