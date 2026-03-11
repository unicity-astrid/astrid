//! # Astrid User Identity - Cross-Platform Identity Management
//!
//! This module provides a unified identity system that maps platform-specific
//! user accounts to a single canonical internal identity: [`AstridUserId`].
//!
//! ## Why Identity Mapping Matters
//!
//! Astrid is designed to be deployed across multiple uplinks simultaneously.
//! The same agent runtime can serve a user through any number of platforms
//! (Telegram, Discord, CLI, web, custom) - all at once. Without identity
//! mapping, each uplink sees a disconnected stranger. With it, the system
//! recognises that the same human is speaking, regardless of which platform
//! they're using.
//!
//! ## Architecture
//!
//! The identity system has two layers:
//!
//! **Layer 1 - Canonical Identity ([`AstridUserId`]):**
//! A UUID-based internal identifier, optionally bound to an ed25519 public key
//! for cryptographic signing. This is the single source of truth for "who is
//! this person" across the entire system.
//!
//! **Layer 2 - Platform Links ([`PlatformLink`]):**
//! Each platform account is linked to exactly one `AstridUserId` via a
//! [`PlatformLink`]. The platform is identified by a simple string (e.g.
//! "discord", "telegram", "cli"). Core doesn't know or care what platforms
//! exist - that's the uplink's business.
//!
//! ## Key Types
//!
//! - [`AstridUserId`] - Canonical internal user identity (UUID + optional ed25519 key)
//! - [`PlatformLink`] - Binds a platform account to an `AstridUserId`
//! - [`LinkVerificationMethod`] - How a cross-platform link was verified
//! - [`PendingLinkCode`] - Time-limited code for cross-platform verification
//! - [`IdentityStore`] - Async trait for identity storage and resolution
//! - [`InMemoryIdentityStore`] - Reference implementation for testing
//! - [`normalize_platform`] - Trim + lowercase for platform name normalization
//!
//! ## Example
//!
//! ```rust
//! use astrid_core::identity::{AstridUserId, PlatformLink, LinkVerificationMethod};
//!
//! let user = AstridUserId::new().with_display_name("Alice");
//!
//! let link = PlatformLink::new(
//!     user.id,
//!     "telegram",
//!     "98765",
//!     LinkVerificationMethod::InitialCreation,
//!     true,
//! );
//!
//! assert_eq!(link.astrid_id, user.id);
//! assert_eq!(link.platform, "telegram");
//! ```

/// Error types for identity management.
pub(crate) mod error;
/// Trait definitions for identity storage.
pub(crate) mod store;
/// Core identity types.
pub(crate) mod types;

pub use error::{IdentityError, IdentityResult};
pub use store::{IdentityStore, InMemoryIdentityStore};
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_astrid_user_id_creation() {
        let user1 = AstridUserId::new();
        let user2 = AstridUserId::new();
        assert_ne!(user1.id, user2.id);
    }

    #[test]
    fn test_astrid_user_id_display() {
        let user = AstridUserId::new();
        let display = user.to_string();
        assert!(display.starts_with("user:"));

        let user_with_name = AstridUserId::new().with_display_name("Alice");
        let display = user_with_name.to_string();
        assert!(display.starts_with("Alice("));
    }

    #[test]
    fn normalize_platform_trims_and_lowercases() {
        assert_eq!(normalize_platform("Discord"), "discord");
        assert_eq!(normalize_platform("  TELEGRAM  "), "telegram");
        assert_eq!(normalize_platform("matrix"), "matrix");
        assert_eq!(normalize_platform("  Matrix  "), "matrix");
    }

    #[test]
    fn test_link_verification_method_display() {
        let method = LinkVerificationMethod::InitialCreation;
        assert_eq!(method.to_string(), "initial_creation");

        let method = LinkVerificationMethod::CodeVerification {
            verified_via: "discord".to_string(),
        };
        assert_eq!(method.to_string(), "code_via:discord");
    }

    #[tokio::test]
    async fn test_in_memory_identity_store() {
        let store = InMemoryIdentityStore::new();

        let user = store.create_identity("discord", "123456").await.unwrap();

        let resolved = store.resolve("discord", "123456").await.unwrap();
        assert_eq!(resolved.id, user.id);

        let by_id = store.get_by_id(user.id).await.unwrap();
        assert_eq!(by_id.id, user.id);
    }

    #[tokio::test]
    async fn test_resolve_case_insensitive() {
        let store = InMemoryIdentityStore::new();

        let user = store.create_identity("Telegram", "123").await.unwrap();

        // Resolve via different casing
        let resolved = store.resolve("telegram", "123").await;
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().id, user.id);

        let resolved = store.resolve("TELEGRAM", "123").await;
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().id, user.id);
    }

    #[tokio::test]
    async fn test_cross_platform_linking() {
        let store = InMemoryIdentityStore::new();

        let user = store
            .create_identity("discord", "discord_123")
            .await
            .unwrap();

        let code = store
            .generate_link_code(user.id, "whatsapp", "whatsapp_456")
            .await
            .unwrap();

        let link = store.verify_link_code(&code, "discord").await.unwrap();

        assert_eq!(link.astrid_id, user.id);
        assert_eq!(link.platform, "whatsapp");

        let resolved = store.resolve("whatsapp", "whatsapp_456").await.unwrap();
        assert_eq!(resolved.id, user.id);

        let links = store.get_links(user.id).await;
        assert_eq!(links.len(), 2);
    }

    #[tokio::test]
    async fn test_duplicate_link_rejected() {
        let store = InMemoryIdentityStore::new();

        store.create_identity("discord", "123").await.unwrap();

        let result = store.create_identity("discord", "123").await;
        assert!(matches!(
            result,
            Err(IdentityError::PlatformAlreadyLinked { .. })
        ));
    }

    #[tokio::test]
    async fn test_duplicate_detection_cross_case() {
        let store = InMemoryIdentityStore::new();

        store.create_identity("Telegram", "123").await.unwrap();

        // Same platform, different case - must be rejected
        let result = store.create_identity("telegram", "123").await;
        assert!(matches!(
            result,
            Err(IdentityError::PlatformAlreadyLinked { .. })
        ));
    }

    #[tokio::test]
    async fn test_remove_link_case_insensitive() {
        let store = InMemoryIdentityStore::new();

        store.create_identity("Telegram", "123").await.unwrap();

        let result = store.remove_link("telegram", "123").await;
        assert!(result.is_ok());

        let resolved = store.resolve("Telegram", "123").await;
        assert!(resolved.is_none());
    }
}
