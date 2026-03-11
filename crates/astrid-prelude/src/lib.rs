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
//! // - (astrid-llm removed - LLM interaction now through capsules)
//! // - astrid-events (event bus)
//! // - astrid-hooks (hook system)
//! // - astrid-workspace (boundaries)
//! // - astrid-telemetry (logging, tracing)
//! // - astrid-kernel (daemon layer)
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

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

// Re-export all crate preludes
pub use astrid_audit::prelude::*;
pub use astrid_capabilities::prelude::*;
pub use astrid_core::prelude::*;
pub use astrid_crypto::prelude::*;
pub use astrid_events::prelude::*;
pub use astrid_hooks::prelude::*;

pub use astrid_mcp::prelude::*;
pub use astrid_telemetry::prelude::*;
pub use astrid_workspace::prelude::*;
