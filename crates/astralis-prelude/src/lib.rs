//! Unified prelude for the Astralis secure agent runtime SDK.
//!
//! This crate provides a single import to bring in all commonly used types
//! from across the Astralis SDK. Use this when you need types from multiple
//! crates without managing individual imports.
//!
//! # Usage
//!
//! ```rust,ignore
//! use astralis_prelude::*;
//!
//! // Now you have access to types from:
//! // - astralis-core (Frontend, errors, identity)
//! // - astralis-crypto (KeyPair, Signature, hashing)
//! // - astralis-capabilities (tokens, stores)
//! // - astralis-audit (logging, verification)
//! // - astralis-mcp (client, tools, servers)
//! // - astralis-runtime (AgentRuntime, sessions)
//! // - astralis-llm (providers, messages)
//! // - astralis-events (event bus)
//! // - astralis-hooks (hook system)
//! // - astralis-workspace (boundaries)
//! // - astralis-telemetry (logging, tracing)
//! // - astralis-gateway (daemon layer)
//! ```
//!
//! # Per-Crate Preludes
//!
//! If you only need types from specific crates, use their individual preludes:
//!
//! ```rust,ignore
//! use astralis_core::prelude::*;
//! use astralis_crypto::prelude::*;
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use astralis_prelude::*;
//!
//! # async fn example() -> RuntimeResult<()> {
//! // Create crypto keys
//! let runtime_key = KeyPair::generate();
//! let audit_key = KeyPair::generate();
//!
//! // Set up audit logging
//! let audit = AuditLog::in_memory(audit_key);
//!
//! // Create MCP client
//! let mcp = McpClient::from_default_config()?;
//!
//! // Create LLM provider
//! let llm = ClaudeProvider::new(ProviderConfig::new("api-key", "claude-sonnet-4-20250514"));
//!
//! // Create runtime
//! let home = astralis_core::dirs::AstralisHome::resolve()?;
//! let sessions = SessionStore::from_home(&home);
//! let runtime = AgentRuntime::new(
//!     llm,
//!     mcp,
//!     audit,
//!     sessions,
//!     runtime_key,
//!     RuntimeConfig::default(),
//! );
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

// Re-export all crate preludes
pub use astralis_audit::prelude::*;
pub use astralis_capabilities::prelude::*;
pub use astralis_core::prelude::*;
pub use astralis_crypto::prelude::*;
pub use astralis_events::prelude::*;
pub use astralis_gateway::prelude::*;
pub use astralis_hooks::prelude::*;
pub use astralis_llm::prelude::*;
pub use astralis_mcp::prelude::*;
pub use astralis_runtime::prelude::*;
pub use astralis_telemetry::prelude::*;
pub use astralis_workspace::prelude::*;
