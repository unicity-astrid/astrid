//! `AstridClientHandler` and bridge channel types.
//!
//! Split into focused submodules:
//! - [`bridge`] — bridge channel types from plugin subprocess output
//! - [`notice`] — `ServerNotice` and size constants
//! - [`handler`] — `AstridClientHandler` struct, builders, and core methods
//! - [`helpers`] — pure helper fns for inbound message processing
//! - [`rmcp_impl`] — `impl rmcp::ClientHandler for AstridClientHandler`

mod bridge;
mod handler;
mod helpers;
mod notice;
mod rmcp_impl;

#[cfg(test)]
mod tests;

pub use bridge::{BridgeChannelCapabilities, BridgeChannelDefinition, BridgeChannelInfo};
pub use handler::AstridClientHandler;
pub use notice::ServerNotice;
