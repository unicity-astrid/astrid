//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_capabilities::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust
//! use astrid_capabilities::prelude::*;
//! use astrid_crypto::KeyPair;
//! use astrid_core::Permission;
//!
//! // Create a capability store
//! let store = CapabilityStore::in_memory();
//!
//! // Create a token
//! let runtime_key = KeyPair::generate();
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
//! // Add and check capability
//! store.add(token).unwrap();
//! assert!(store.has_capability("mcp://filesystem:read_file", Permission::Invoke));
//! ```

// Errors
pub use crate::{CapabilityError, CapabilityResult};

// Token types
pub use crate::{AuditEntryId, CapabilityToken, TokenBuilder, TokenScope};

// Resource patterns
pub use crate::{ResourcePattern, ResourceUri};

// Store and validation
pub use crate::CapabilityStore;
pub use crate::{AuthorizationResult, CapabilityValidator, MultiPermissionCheck};
