//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astralis_gateway::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,ignore
//! use astralis_gateway::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> GatewayResult<()> {
//!     let config = GatewayConfig::load("~/.astralis/gateway.toml")?;
//!     let runtime = GatewayRuntime::new(config)?;
//!
//!     // Check health
//!     let health = runtime.health().await;
//!     if health.state == HealthState::Healthy {
//!         runtime.run().await?;
//!     }
//!
//!     Ok(())
//! }
//! ```

// Errors
pub use crate::{GatewayError, GatewayResult};

// Configuration
pub use crate::{AgentConfig, GatewayConfig, ModelConfig, SessionConfig, TimeoutConfig};

// Runtime
pub use crate::GatewayRuntime;

// Health checks
pub use crate::{HealthCheck, HealthState, HealthStatus};

// Agent management
pub use crate::{AgentHandle, AgentManager, AgentStatus};

// Routing
pub use crate::{ChannelBinding, MessageRouter};

// Secrets
pub use crate::Secrets;

// State persistence
pub use crate::{PendingApproval, PersistedState, QueuedTask, SubAgentState};

// Subagent management
pub use crate::{SubAgentHandle, SubAgentId, SubAgentPool, SubAgentPoolStats, SubAgentStatus};
