//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_crypto::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust
//! use astrid_crypto::prelude::*;
//!
//! // Generate a key pair
//! let keypair = KeyPair::generate();
//!
//! // Sign and verify
//! let message = b"hello";
//! let signature = keypair.sign(message);
//! assert!(keypair.verify(message, &signature).is_ok());
//!
//! // Hash content
//! let hash = ContentHash::hash(message);
//! ```

// Errors
pub use crate::{CryptoError, CryptoResult};

// Key types
pub use crate::{KeyId, KeyPair, PublicKey};

// Signature
pub use crate::Signature;

// Signature verification
pub use crate::SignatureVerifier;

// Hashing
pub use crate::ContentHash;
