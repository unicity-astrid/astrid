//! Astrid Capabilities - Cryptographically signed authorization tokens.
//!
//! This crate provides:
//! - Capability tokens with ed25519 signatures
//! - Resource patterns with glob matching
//! - Session and persistent token storage
//! - Token validation and authorization checking
//!
//! # Security Model
//!
//! Every capability token is:
//! - Signed by the runtime's ed25519 key
//! - Linked to the approval audit entry that created it
//! - Time-bounded (optional expiration)
//! - Scoped (session or persistent)
//!
//! # Example
//!
//! ```
//! use astrid_capabilities::{
//!     CapabilityToken, CapabilityStore, ResourcePattern, TokenScope, AuditEntryId,
//! };
//! use astrid_core::Permission;
//! use astrid_crypto::KeyPair;
//!
//! // Create a capability store
//! let store = CapabilityStore::in_memory();
//!
//! // Runtime key for signing
//! let runtime_key = KeyPair::generate();
//!
//! // Create a capability token
//! let token = CapabilityToken::create(
//!     ResourcePattern::new("mcp://filesystem:*").unwrap(),
//!     vec![Permission::Invoke],
//!     TokenScope::Session,
//!     runtime_key.key_id(),
//!     AuditEntryId::new(),
//!     &runtime_key,
//!     None,
//! );
//!
//! // Add to store
//! store.add(token).unwrap();
//!
//! // Check capability
//! assert!(store.has_capability("mcp://filesystem:read_file", Permission::Invoke));
//! ```

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod prelude;

mod error;
mod pattern;
mod store;
mod token;
mod validator;

pub use error::{CapabilityError, CapabilityResult};
pub use pattern::{ResourcePattern, ResourceUri};
pub use store::CapabilityStore;
pub use token::{AuditEntryId, CapabilityToken, TokenBuilder, TokenScope};
pub use validator::{AuthorizationResult, CapabilityValidator, MultiPermissionCheck};
