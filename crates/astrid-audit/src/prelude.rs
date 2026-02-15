//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_audit::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust
//! use astrid_audit::prelude::*;
//! use astrid_core::SessionId;
//! use astrid_crypto::KeyPair;
//!
//! // Create an audit log
//! let runtime_key = KeyPair::generate();
//! let user_id = runtime_key.key_id();
//! let log = AuditLog::in_memory(runtime_key);
//!
//! // Record an action
//! let session_id = SessionId::new();
//! let entry_id = log.append(
//!     session_id.clone(),
//!     AuditAction::SessionStarted {
//!         user_id,
//!         frontend: "cli".to_string(),
//!     },
//!     AuthorizationProof::System {
//!         reason: "session start".to_string(),
//!     },
//!     AuditOutcome::success(),
//! ).unwrap();
//!
//! // Verify chain integrity
//! let result = log.verify_chain(&session_id).unwrap();
//! assert!(result.valid);
//! ```

// Errors
pub use crate::{AuditError, AuditResult};

// Entry types
pub use crate::{ApprovalScope, AuditAction, AuditEntry, AuditOutcome, AuthorizationProof};

// Log and verification
pub use crate::{AuditBuilder, AuditLog, ChainIssue, ChainVerificationResult};

// Storage
pub use crate::{AuditStorage, SurrealKvAuditStorage};

// Re-export from capabilities
pub use crate::AuditEntryId;
