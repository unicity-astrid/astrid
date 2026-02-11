//! Astralis Runtime - Agent orchestration and session management.
//!
//! This crate provides:
//! - Agent runtime with LLM, MCP, and security integration
//! - Session management with persistence
//! - Context management with auto-summarization
//!
//! # Architecture
//!
//! The runtime coordinates:
//! - LLM provider for language model interactions
//! - MCP client for tool execution
//! - Capability store for authorization
//! - Audit log for security logging
//!
//! # Example
//!
//! ```rust,no_run
//! use astralis_runtime::{AgentRuntime, RuntimeConfig, SessionStore};
//! use astralis_llm::{ClaudeProvider, ProviderConfig};
//! use astralis_mcp::McpClient;
//! use astralis_audit::AuditLog;
//! use astralis_crypto::KeyPair;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create components
//! let llm = ClaudeProvider::new(ProviderConfig::new("api-key", "claude-sonnet-4-20250514"));
//! let mcp = McpClient::from_default_config()?;
//! let audit_key = KeyPair::generate();
//! let runtime_key = KeyPair::generate();
//! let audit = AuditLog::in_memory(audit_key);
//! let home = astralis_core::dirs::AstralisHome::resolve()?;
//! let sessions = SessionStore::from_home(&home);
//!
//! // Create runtime
//! let runtime = AgentRuntime::new(
//!     llm,
//!     mcp,
//!     audit,
//!     sessions,
//!     runtime_key,
//!     RuntimeConfig::default(),
//! );
//!
//! // Create a session
//! let mut session = runtime.create_session(None);
//!
//! // Run a turn (would need a Frontend implementation)
//! // runtime.run_turn_streaming(&mut session, "Hello!", &frontend).await?;
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod config_bridge;
pub mod prelude;

mod context;
mod error;
mod runtime;
mod session;
mod store;
pub mod subagent;
pub mod subagent_executor;

pub use context::{ContextManager, ContextStats, SummarizationResult};
pub use error::{RuntimeError, RuntimeResult};
pub use runtime::{AgentRuntime, RuntimeConfig};
pub use session::{AgentSession, GitState, SerializableSession, SessionMetadata};
pub use store::{SessionStore, SessionSummary};
pub use subagent::{SubAgentHandle, SubAgentId, SubAgentPool, SubAgentPoolStats, SubAgentStatus};
pub use subagent_executor::SubAgentExecutor;

// Re-export workspace types for convenience
pub use astralis_workspace::{self, WorkspaceBoundary, WorkspaceConfig, WorkspaceMode};

// Re-export tools types for convenience
pub use astralis_tools::{self, ToolContext, ToolRegistry, build_system_prompt};
