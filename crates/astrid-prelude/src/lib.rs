//! Unified prelude for the Astrid secure agent runtime.
//!
//! This crate provides a single import to bring in all commonly used types
//! from across Astrid. Use this when you need types from multiple
//! crates without managing individual imports.
//!
//! # Usage
//!
//! ```rust,ignore
//! use astrid_prelude::*;
//!
//! // Now you have access to types from:
//! // - astrid-core (Frontend, errors, identity)
//! // - astrid-crypto (KeyPair, Signature, hashing)
//! // - astrid-capabilities (tokens, stores)
//! // - astrid-audit (logging, verification)
//! // - astrid-mcp (client, tools, servers)
//! // - astrid-runtime (AgentRuntime, sessions)
//! // - astrid-llm (providers, messages)
//! // - astrid-events (event bus)
//! // - astrid-hooks (hook system)
//! // - astrid-workspace (boundaries)
//! // - astrid-telemetry (logging, tracing)
//! // - astrid-gateway (daemon layer)
//! ```
//!
//! # Per-Crate Preludes
//!
//! If you only need types from specific crates, use their individual preludes:
//!
//! ```rust,ignore
//! use astrid_core::prelude::*;
//! use astrid_crypto::prelude::*;
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use astrid_prelude::*;
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
//! let home = astrid_core::dirs::AstridHome::resolve()?;
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
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

// Re-export all crate preludes
pub use astrid_audit::prelude::*;
pub use astrid_capabilities::prelude::*;
pub use astrid_core::prelude::*;
pub use astrid_crypto::prelude::*;
pub use astrid_events::prelude::*;
pub use astrid_gateway::prelude::*;
pub use astrid_hooks::prelude::*;
pub use astrid_llm::prelude::*;
pub use astrid_mcp::prelude::*;
pub use astrid_runtime::prelude::*;
pub use astrid_telemetry::prelude::*;
pub use astrid_workspace::prelude::*;
