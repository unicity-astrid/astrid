//! Astrid Gateway - Daemon layer for the Astrid secure agent runtime.
//!
//! This crate provides a daemon wrapper around `astrid-runtime` that handles:
//! - Configuration loading and hot-reload
//! - Multi-agent management
//! - Message routing between frontends and agents
//! - State persistence and checkpointing
//! - Health checks and monitoring
//! - Graceful shutdown
//!
//! # Architecture
//!
//! ```text
//! astrid-kernel (daemon layer)
//! ├── Config loading & hot-reload
//! ├── Multi-agent management
//! ├── Message routing
//! ├── State persistence & checkpointing
//! ├── Health checks
//! ├── Graceful shutdown
//! └── astrid-runtime (orchestration layer)
//!     ├── Session management
//!     ├── Context management
//!     └── astrid-llm (provider layer)
//!         └── astrid-mcp (tool layer)
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use astrid_kernel::{GatewayConfig, GatewayRuntime};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = GatewayConfig::load("~/.astrid/gateway.toml")?;
//!     let runtime = GatewayRuntime::new(config)?;
//!
//!     // Run the gateway (blocks until shutdown)
//!     runtime.run().await?;
//!
//!     Ok(())
//! }
//! ```

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod prelude;

pub mod config;
pub mod config_bridge;
pub mod daemon_frontend;
pub mod error;
pub mod rpc;
pub mod server;

pub use config::{
    AgentConfig, GatewayConfig, ModelConfig, RetrySettings, SessionConfig, TimeoutConfig,
};
pub use error::{GatewayError, GatewayResult};
pub use rpc::{
    AstridRpcClient, CapsuleInfo, DaemonEvent, DaemonStatus, McpServerInfo, SessionInfo, ToolInfo,
};
pub use server::{DaemonServer, DaemonStartOptions};
