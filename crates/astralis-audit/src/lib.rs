//! Astralis Audit - Chain-linked cryptographic audit logging.
//!
//! This crate provides:
//! - Cryptographically signed audit entries
//! - Chain-linked entries (each contains hash of previous)
//! - Persistent storage with `SurrealKV`
//! - Chain integrity verification
//!
//! # Security Model
//!
//! Every audit entry is:
//! - Signed by the runtime's ed25519 key
//! - Linked to the previous entry via content hash
//! - Timestamped
//! - Indexed by session
//!
//! The chain linking provides tamper evidence - any modification
//! to historical entries breaks the chain and is detectable.
//!
//! # Example
//!
//! ```
//! use astralis_audit::{AuditLog, AuditAction, AuditOutcome, AuthorizationProof};
//! use astralis_core::SessionId;
//! use astralis_crypto::KeyPair;
//!
//! // Create an in-memory audit log
//! let runtime_key = KeyPair::generate();
//! let user_id = runtime_key.key_id();
//! let log = AuditLog::in_memory(runtime_key);
//!
//! // Start a session
//! let session_id = SessionId::new();
//!
//! // Record an action
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

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

mod entry;
mod error;
mod log;
mod storage;

pub use entry::{ApprovalScope, AuditAction, AuditEntry, AuditOutcome, AuthorizationProof};
pub use error::{AuditError, AuditResult};
pub use log::{AuditBuilder, AuditLog, ChainIssue, ChainVerificationResult};
pub use storage::{AuditStorage, SurrealKvAuditStorage};

// Re-export AuditEntryId from capabilities for convenience
pub use astralis_capabilities::AuditEntryId;
