//! Astralis Gateway - Daemon layer for the Astralis secure agent runtime.
//!
//! This crate provides a daemon wrapper around `astralis-runtime` that handles:
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
//! astralis-gateway (daemon layer)
//! ├── Config loading & hot-reload
//! ├── Multi-agent management
//! ├── Message routing
//! ├── State persistence & checkpointing
//! ├── Health checks
//! ├── Graceful shutdown
//! └── astralis-runtime (orchestration layer)
//!     ├── Session management
//!     ├── Context management
//!     └── astralis-llm (provider layer)
//!         └── astralis-mcp (tool layer)
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use astralis_gateway::{GatewayConfig, GatewayRuntime};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = GatewayConfig::load("~/.config/astralis/gateway.toml")?;
//!     let runtime = GatewayRuntime::new(config)?;
//!
//!     // Run the gateway (blocks until shutdown)
//!     runtime.run().await?;
//!
//!     Ok(())
//! }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

pub mod config;
pub mod config_bridge;
pub mod daemon_frontend;
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
pub use error::{GatewayError, GatewayResult};
pub use health::{HealthCheck, HealthState, HealthStatus};
pub use manager::{AgentHandle, AgentManager, AgentStatus};
pub use router::{ChannelBinding, MessageRouter};
pub use rpc::{AstralisRpcClient, DaemonEvent, DaemonStatus, McpServerInfo, SessionInfo, ToolInfo};
pub use runtime::GatewayRuntime;
pub use secrets::Secrets;
pub use server::{DaemonServer, DaemonStartOptions};
pub use state::{PendingApproval, PersistedState, QueuedTask, SubAgentState};
pub use subagent::{SubAgentHandle, SubAgentId, SubAgentPool, SubAgentPoolStats, SubAgentStatus};
