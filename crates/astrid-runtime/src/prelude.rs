//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_runtime::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,no_run
//! use astrid_runtime::prelude::*;
//! use astrid_llm::{ClaudeProvider, ProviderConfig};
//! use astrid_mcp::McpClient;
//! use astrid_audit::AuditLog;
//! use astrid_crypto::KeyPair;
//!
//! # async fn example() -> RuntimeResult<()> {
//! // Create components
//! let llm = ClaudeProvider::new(ProviderConfig::new("api-key", "claude-sonnet-4-20250514"));
//! let mcp = McpClient::from_default_config()?;
//! let audit_key = KeyPair::generate();
//! let runtime_key = KeyPair::generate();
//! let audit = AuditLog::in_memory(audit_key);
//! let home = astrid_core::dirs::AstridHome::resolve()?;
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
//! let session = runtime.create_session(None);
//! # Ok(())
//! # }
//! ```

// Errors
pub use crate::{RuntimeError, RuntimeResult};

// Runtime
pub use crate::{AgentRuntime, RuntimeConfig};

// Sessions
pub use crate::{AgentSession, SerializableSession, SessionMetadata};
pub use crate::{SessionStore, SessionSummary};

// Context management
pub use crate::{ContextManager, ContextStats, SummarizationResult};
