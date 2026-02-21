//! MCP plugin implementation.

mod plugin;
mod tool;

pub(crate) mod platform;

pub use plugin::{McpPlugin, create_plugin};
pub mod protocol;
pub mod state;
