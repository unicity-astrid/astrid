//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_kernel::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,ignore
//! use astrid_kernel::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> GatewayResult<()> {
//!     let config = GatewayConfig::load("~/.astrid/gateway.toml")?;
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

// Health checks

// Agent management

// Routing

// Secrets

// State persistence

// Subagent management
