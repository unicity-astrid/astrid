//! Execution engine trait for Composite Capsules.
//!
//! Because a single `Capsule.toml` can define multiple execution units
//! (e.g. a WASM component AND a legacy MCP host process), the OS uses
//! an additive "Composite" architecture. The capsule iterates over its
//! registered engines to handle lifecycle events.

pub mod mcp;
mod static_engine;
pub mod wasm;

pub use mcp::McpHostEngine;
pub use static_engine::StaticEngine;
pub use wasm::WasmEngine;

use async_trait::async_trait;

use crate::context::CapsuleContext;
use crate::error::CapsuleResult;

/// A runtime environment capable of executing capsule logic.
///
/// Examples include `WasmEngine`, `McpHostEngine`, and `StaticEngine`.
#[async_trait]
pub trait ExecutionEngine: Send + Sync {
    /// Load the engine (e.g., spawn the WASM VM or start the Node.js process).
    async fn load(&mut self, ctx: &CapsuleContext) -> CapsuleResult<()>;

    /// Unload the engine (e.g., drop WASM memory or SIGTERM the child process).
    async fn unload(&mut self) -> CapsuleResult<()>;

    /// Extract the inbound receiver if this engine provides one.
    fn take_inbound_rx(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<astrid_core::InboundMessage>> {
        None
    }
}
