//! Shared abstractions for Astrid frontend crates.
//!
//! This crate contains platform-agnostic code used by all frontend
//! implementations (Telegram, Discord, etc.):
//!
//! - [`DaemonClient`] — `WebSocket` JSON-RPC client for the daemon
//! - [`SessionMap`] — generic channel-to-session mapping with turn locking
//! - [`PendingStore`] — TTL-based pending request store
//! - [`format`] — text chunking utilities
//! - [`FrontendCommonError`] — shared error type

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]

pub mod client;
pub mod error;
pub mod format;
pub mod pending;
pub mod session;

/// Prelude re-exports for convenient use.
pub mod prelude {
    pub use crate::client::DaemonClient;
    pub use crate::error::{FrontendCommonError, FrontendCommonResult};
    pub use crate::format::{chunk_text, find_split_point};
    pub use crate::pending::PendingStore;
    pub use crate::session::{ChannelSession, SessionMap, TurnStartResult};
}

// Re-export key types at crate root for convenience.
pub use client::DaemonClient;
pub use error::FrontendCommonError;
pub use pending::PendingStore;
pub use session::SessionMap;
