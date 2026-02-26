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
//! astrid-gateway (daemon layer)
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
//! use astrid_gateway::{GatewayConfig, GatewayRuntime};
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
pub mod discord_proxy;
pub mod error;
pub mod health;
pub mod manager;
pub mod router;
pub mod rpc;
pub mod runtime;
pub mod secrets;
pub mod server;
pub mod state;
pub mod subagent;

pub use config::{
    AgentConfig, GatewayConfig, ModelConfig, RetrySettings, SessionConfig, TimeoutConfig,
};
pub use discord_proxy::{DiscordGatewayProxy, DiscordProxyConfig};
pub use error::{GatewayError, GatewayResult};
pub use health::{HealthCheck, HealthState, HealthStatus};
pub use manager::{AgentHandle, AgentManager, AgentStatus};
pub use router::{ChannelBinding, MessageRouter};
pub use rpc::{
    AstridRpcClient, CapsuleInfo, DaemonEvent, DaemonStatus, McpServerInfo, SessionInfo, ToolInfo,
};
pub use runtime::GatewayRuntime;
pub use secrets::Secrets;
pub use server::{DaemonServer, DaemonStartOptions};
pub use state::{PendingApproval, PersistedState, QueuedTask, SubAgentState};
pub use subagent::{SubAgentHandle, SubAgentId, SubAgentPool, SubAgentPoolStats, SubAgentStatus};
