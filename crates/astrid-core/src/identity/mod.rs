//! # Astrid User Identity — Cross-Frontend Identity Management
//!
//! This module provides a unified identity system that maps platform-specific
//! user accounts (Discord, Telegram, WhatsApp, CLI, etc.) to a single canonical
//! internal identity: [`AstridUserId`].
//!
//! ## Why Identity Mapping Matters
//!
//! Astrid is designed to be deployed across multiple frontends simultaneously.
//! The same agent runtime can serve a user through a Telegram bot, a Discord
//! channel, a CLI session, and a web dashboard — all at once. Without identity
//! mapping, each frontend sees a disconnected stranger. With it, the system
//! recognises that the same human is speaking, regardless of which platform
//! they're using.
//!
//! This has three critical consequences:
//!
//! ### 1. Memory Continuity
//!
//! When the memory system is active, facts learned about a user on one platform
//! carry over to every other platform they've linked. If a user tells the agent
//! their preferred programming language via Discord, that preference is available
//! when they later interact via Telegram. Without identity mapping, each frontend
//! would build an isolated, incomplete picture of the same person.
//!
//! ### 2. Multi-Tenant Context Isolation
//!
//! Astrid is tenanted by design. When deployed into a shared environment (a
//! Discord server, a Telegram group), the system isolates context per-user
//! and per-environment. Identity mapping is what makes this possible — it
//! distinguishes User A from User B within the same channel, and User A in
//! Channel X from User A in Channel Y.
//!
//! This isolation is enforced through [`ContextIdentifier`](crate::input::ContextIdentifier),
//! which combines the frontend type, a context ID (channel/group/chat), and the
//! resolved [`AstridUserId`]. Together these form the boundaries for session
//! state, approval history, capability tokens, and eventually memory retrieval.
//!
//! ### 3. Unified Security and Audit
//!
//! Capability tokens, approval history, budget tracking, and audit entries are
//! all anchored to the canonical [`AstridUserId`]. When a user grants "Allow
//! Always" on one platform, the resulting capability token is bound to their
//! internal identity — not to a transient platform-specific ID. Audit trails
//! can trace actions back to a single person across all their linked accounts.
//!
//! ## Architecture
//!
//! The identity system has two layers:
//!
//! **Layer 1 — Canonical Identity ([`AstridUserId`]):**
//! A UUID-based internal identifier, optionally bound to an ed25519 public key
//! for cryptographic signing. This is the single source of truth for "who is
//! this person" across the entire system.
//!
//! **Layer 2 — Platform Links ([`FrontendLink`]):**
//! Each platform account (e.g., Discord user `123456789`, Telegram user `987654`)
//! is linked to exactly one `AstridUserId` via a [`FrontendLink`]. Links are
//! verified through one of three methods (see [`LinkVerificationMethod`]):
//! initial creation, cross-platform code verification, or admin linking.
//!
//! ```text
//!                    ┌─────────────────────┐
//!                    │   AstridUserId     │
//!                    │   (UUID + pubkey)     │
//!                    └────────┬────────────┘
//!                             │
//!              ┌──────────────┼──────────────┐
//!              │              │              │
//!     ┌────────▼───┐  ┌──────▼─────┐  ┌─────▼──────┐
//!     │ Discord    │  │ Telegram   │  │ CLI        │
//!     │ "12345"    │  │ "98765"    │  │ "cli_user" │
//!     └────────────┘  └────────────┘  └────────────┘
//!       FrontendLink    FrontendLink    FrontendLink
//! ```
//!
//! ## Cross-Frontend Linking
//!
//! When a user is already known on one platform and wants to link a second,
//! the [`IdentityStore`] provides a verification flow:
//!
//! 1. User requests a link from the new platform (e.g., Telegram).
//! 2. A 9-digit [`PendingLinkCode`] is generated (5-minute TTL).
//! 3. User enters that code on the already-verified platform (e.g., Discord).
//! 4. If the code matches, a new [`FrontendLink`] is created, binding the
//!    Telegram account to the same [`AstridUserId`].
//!
//! ## For Frontend Implementors
//!
//! Every [`Frontend`](crate::frontend::Frontend) implementation should resolve
//! identity on first contact with a user. The typical pattern:
//!
//! 1. Extract the platform-specific user ID from the incoming message.
//! 2. Call [`IdentityStore::resolve`] with the [`FrontendType`] and platform ID.
//! 3. If `None`, this is a new user — call [`IdentityStore::create_identity`]
//!    to mint a fresh [`AstridUserId`] with an initial [`FrontendLink`].
//! 4. Populate [`FrontendUser::astrid_id`](crate::frontend::FrontendUser::astrid_id)
//!    with the resolved UUID.
//! 5. Include the resolved identity in [`FrontendContext`](crate::frontend::FrontendContext)
//!    so downstream systems (sessions, approval, audit) operate on the canonical ID.
//!
//! ## Key Types
//!
//! - [`AstridUserId`] — Canonical internal user identity (UUID + optional ed25519 key)
//! - [`FrontendLink`] — Binds a platform account to an `AstridUserId`
//! - [`FrontendType`] — Enum of supported platforms (Discord, Telegram, WhatsApp, etc.)
//! - [`LinkVerificationMethod`] — How a cross-platform link was verified
//! - [`PendingLinkCode`] — Time-limited code for cross-frontend verification
//! - [`IdentityStore`] — Async trait for identity storage and resolution
//! - [`InMemoryIdentityStore`] — Reference implementation for testing
//!
//! ## Example
//!
//! ```rust
//! use astrid_core::identity::{AstridUserId, FrontendType, FrontendLink, LinkVerificationMethod};
//! use uuid::Uuid;
//!
//! // Create a canonical identity
//! let user = AstridUserId::new().with_display_name("Alice");
//!
//! // Link a Telegram account to it
//! let link = FrontendLink::new(
//!     user.id,
//!     FrontendType::Telegram,
//!     "98765",
//!     LinkVerificationMethod::InitialCreation,
//!     true, // primary
//! );
//!
//! assert_eq!(link.astrid_id, user.id);
//! assert_eq!(link.frontend, FrontendType::Telegram);
//! ```

// Allow "WhatsApp" in docs - it's a product name, not code
#![allow(clippy::doc_markdown)]

pub mod error;
pub mod store;
pub mod types;

pub use error::{IdentityError, IdentityResult};
pub use store::{IdentityStore, InMemoryIdentityStore};
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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
    fn test_frontend_type_display() {
        assert_eq!(FrontendType::Discord.to_string(), "discord");
        assert_eq!(FrontendType::WhatsApp.to_string(), "whatsapp");
        assert_eq!(
            FrontendType::Custom("matrix".to_string()).to_string(),
            "custom:matrix"
        );
    }

    #[test]
    fn canonical_name_known_variants() {
        assert_eq!(FrontendType::Discord.canonical_name(), "discord");
        assert_eq!(FrontendType::WhatsApp.canonical_name(), "whatsapp");
        assert_eq!(FrontendType::Telegram.canonical_name(), "telegram");
        assert_eq!(FrontendType::Slack.canonical_name(), "slack");
        assert_eq!(FrontendType::Web.canonical_name(), "web");
        assert_eq!(FrontendType::Cli.canonical_name(), "cli");
    }

    #[test]
    fn canonical_name_collapses_known_aliases() {
        // Custom strings matching a known variant collapse to that variant's key.
        assert_eq!(
            FrontendType::Custom("telegram".into()).canonical_name(),
            "telegram"
        );
        assert_eq!(
            FrontendType::Custom("Telegram".into()).canonical_name(),
            "telegram"
        );
        assert_eq!(
            FrontendType::Custom("DISCORD".into()).canonical_name(),
            "discord"
        );
        // "whats_app" (underscore variant) also collapses to "whatsapp".
        assert_eq!(
            FrontendType::Custom("whats_app".into()).canonical_name(),
            "whatsapp"
        );
        assert_eq!(
            FrontendType::Custom("Whats_App".into()).canonical_name(),
            "whatsapp"
        );
    }

    #[test]
    fn canonical_name_trims_whitespace() {
        // Leading/trailing whitespace is stripped before matching.
        assert_eq!(
            FrontendType::Custom("  telegram  ".into())
                .canonical_name()
                .as_ref(),
            "telegram"
        );
        assert_eq!(
            FrontendType::Custom("  matrix  ".into())
                .canonical_name()
                .as_ref(),
            "matrix"
        );
    }

    #[test]
    fn canonical_name_lowercases_unknown_custom() {
        assert_eq!(
            FrontendType::Custom("matrix".into())
                .canonical_name()
                .as_ref(),
            "matrix"
        );
        assert_eq!(
            FrontendType::Custom("MATRIX".into())
                .canonical_name()
                .as_ref(),
            "matrix"
        );
        assert_eq!(
            FrontendType::Custom("Matrix".into())
                .canonical_name()
                .as_ref(),
            "matrix"
        );
    }

    #[test]
    fn is_same_platform_known_vs_custom_alias() {
        assert!(FrontendType::Telegram.is_same_platform(&FrontendType::Custom("telegram".into())));
        assert!(FrontendType::Telegram.is_same_platform(&FrontendType::Custom("Telegram".into())));
        assert!(FrontendType::Discord.is_same_platform(&FrontendType::Custom("DISCORD".into())));
        assert!(FrontendType::WhatsApp.is_same_platform(&FrontendType::Custom("whats_app".into())));
    }

    #[test]
    fn is_same_platform_whitespace_normalized() {
        assert!(
            FrontendType::Telegram.is_same_platform(&FrontendType::Custom("  telegram  ".into()))
        );
        assert!(
            FrontendType::Custom("  matrix  ".into())
                .is_same_platform(&FrontendType::Custom("matrix".into()))
        );
    }

    #[test]
    fn is_same_platform_custom_case_insensitive() {
        assert!(
            FrontendType::Custom("MATRIX".into())
                .is_same_platform(&FrontendType::Custom("matrix".into()))
        );
        assert!(
            FrontendType::Custom("Matrix".into())
                .is_same_platform(&FrontendType::Custom("MATRIX".into()))
        );
    }

    #[test]
    fn is_same_platform_different_platforms() {
        assert!(!FrontendType::Discord.is_same_platform(&FrontendType::Telegram));
        assert!(
            !FrontendType::Custom("matrix".into())
                .is_same_platform(&FrontendType::Custom("signal".into()))
        );
    }

    #[test]
    fn is_same_platform_empty_custom_values_are_equal() {
        // Two empty/whitespace-only Custom values both canonical to "" and compare equal.
        // The WASM boundary rejects these, but the public API should have defined behavior.
        assert!(
            FrontendType::Custom(String::new())
                .is_same_platform(&FrontendType::Custom(String::new()))
        );
        assert!(
            FrontendType::Custom(String::new())
                .is_same_platform(&FrontendType::Custom("   ".into()))
        );
    }

    #[test]
    fn test_link_verification_method_display() {
        let method = LinkVerificationMethod::InitialCreation;
        assert_eq!(method.to_string(), "initial_creation");

        let method = LinkVerificationMethod::CodeVerification {
            verified_via: FrontendType::Discord,
        };
        assert_eq!(method.to_string(), "code_via:discord");
    }

    #[tokio::test]
    async fn test_in_memory_identity_store() {
        let store = InMemoryIdentityStore::new();

        // Create identity
        let user = store
            .create_identity(FrontendType::Discord, "123456")
            .await
            .unwrap();

        // Resolve should work
        let resolved = store
            .resolve(&FrontendType::Discord, "123456")
            .await
            .unwrap();
        assert_eq!(resolved.id, user.id);

        // Get by ID should work
        let by_id = store.get_by_id(user.id).await.unwrap();
        assert_eq!(by_id.id, user.id);
    }

    #[tokio::test]
    async fn test_cross_frontend_linking() {
        let store = InMemoryIdentityStore::new();

        // Create identity on Discord
        let user = store
            .create_identity(FrontendType::Discord, "discord_123")
            .await
            .unwrap();

        // Generate link code
        let code = store
            .generate_link_code(user.id, FrontendType::WhatsApp, "whatsapp_456")
            .await
            .unwrap();

        // Verify the code (simulates entering code on Discord)
        let link = store
            .verify_link_code(&code, FrontendType::Discord)
            .await
            .unwrap();

        assert_eq!(link.astrid_id, user.id);
        assert_eq!(link.frontend, FrontendType::WhatsApp);

        // Now WhatsApp should resolve to the same identity
        let resolved = store
            .resolve(&FrontendType::WhatsApp, "whatsapp_456")
            .await
            .unwrap();
        assert_eq!(resolved.id, user.id);

        // Get all links
        let links = store.get_links(user.id).await;
        assert_eq!(links.len(), 2); // Discord + WhatsApp
    }

    #[tokio::test]
    async fn test_duplicate_link_rejected() {
        let store = InMemoryIdentityStore::new();

        // Create identity
        store
            .create_identity(FrontendType::Discord, "123")
            .await
            .unwrap();

        // Try to create another identity with same frontend user
        let result = store.create_identity(FrontendType::Discord, "123").await;

        assert!(matches!(
            result,
            Err(IdentityError::FrontendAlreadyLinked { .. })
        ));
    }

    // --- FrontendType::FromStr ---

    #[test]
    fn frontend_type_from_str_known_variants() {
        assert_eq!(
            FrontendType::from_str("discord").unwrap(),
            FrontendType::Discord
        );
        assert_eq!(
            FrontendType::from_str("telegram").unwrap(),
            FrontendType::Telegram
        );
        assert_eq!(
            FrontendType::from_str("whatsapp").unwrap(),
            FrontendType::WhatsApp
        );
        assert_eq!(
            FrontendType::from_str("whats_app").unwrap(),
            FrontendType::WhatsApp
        );
        assert_eq!(
            FrontendType::from_str("slack").unwrap(),
            FrontendType::Slack
        );
        assert_eq!(FrontendType::from_str("web").unwrap(), FrontendType::Web);
        assert_eq!(FrontendType::from_str("cli").unwrap(), FrontendType::Cli);
    }

    #[test]
    fn frontend_type_from_str_case_insensitive() {
        assert_eq!(
            FrontendType::from_str("Discord").unwrap(),
            FrontendType::Discord
        );
        assert_eq!(
            FrontendType::from_str("TELEGRAM").unwrap(),
            FrontendType::Telegram
        );
        assert_eq!(
            FrontendType::from_str("WhatsApp").unwrap(),
            FrontendType::WhatsApp
        );
    }

    #[test]
    fn frontend_type_from_str_custom_fallback() {
        let ft = FrontendType::from_str("matrix").unwrap();
        assert_eq!(ft, FrontendType::Custom("matrix".to_string()));
    }

    #[test]
    fn frontend_type_from_str_trims_whitespace() {
        assert_eq!(
            FrontendType::from_str("  discord  ").unwrap(),
            FrontendType::Discord
        );
        let ft = FrontendType::from_str("  matrix  ").unwrap();
        assert_eq!(ft, FrontendType::Custom("matrix".to_string()));
    }

    // --- FrontendType PartialEq / Hash normalization ---

    #[test]
    fn frontend_type_eq_normalized_known_vs_custom() {
        // Known variant equals Custom alias of the same platform.
        assert_eq!(
            FrontendType::Telegram,
            FrontendType::Custom("telegram".into())
        );
        assert_eq!(
            FrontendType::Telegram,
            FrontendType::Custom("Telegram".into())
        );
        assert_eq!(
            FrontendType::Telegram,
            FrontendType::Custom("TELEGRAM".into())
        );
        assert_eq!(
            FrontendType::Discord,
            FrontendType::Custom("discord".into())
        );
        assert_eq!(
            FrontendType::Discord,
            FrontendType::Custom("DISCORD".into())
        );
        assert_eq!(
            FrontendType::WhatsApp,
            FrontendType::Custom("whatsapp".into())
        );
        assert_eq!(
            FrontendType::WhatsApp,
            FrontendType::Custom("whats_app".into())
        );
        assert_eq!(FrontendType::Slack, FrontendType::Custom("slack".into()));
        assert_eq!(FrontendType::Web, FrontendType::Custom("web".into()));
        assert_eq!(FrontendType::Cli, FrontendType::Custom("cli".into()));
    }

    #[test]
    fn frontend_type_eq_different_platforms() {
        assert_ne!(FrontendType::Telegram, FrontendType::Discord);
        assert_ne!(
            FrontendType::Custom("matrix".into()),
            FrontendType::Custom("signal".into())
        );
        assert_ne!(
            FrontendType::Custom("matrix".into()),
            FrontendType::Telegram
        );
    }

    #[test]
    fn frontend_type_hash_normalized() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn compute_hash(ft: &FrontendType) -> u64 {
            let mut hasher = DefaultHasher::new();
            ft.hash(&mut hasher);
            hasher.finish()
        }

        // Known variant and Custom alias must produce identical hashes.
        assert_eq!(
            compute_hash(&FrontendType::Telegram),
            compute_hash(&FrontendType::Custom("telegram".into()))
        );
        assert_eq!(
            compute_hash(&FrontendType::Telegram),
            compute_hash(&FrontendType::Custom("TELEGRAM".into()))
        );
        assert_eq!(
            compute_hash(&FrontendType::Discord),
            compute_hash(&FrontendType::Custom("discord".into()))
        );
        assert_eq!(
            compute_hash(&FrontendType::WhatsApp),
            compute_hash(&FrontendType::Custom("whats_app".into()))
        );
        // Custom case-insensitive
        assert_eq!(
            compute_hash(&FrontendType::Custom("matrix".into())),
            compute_hash(&FrontendType::Custom("MATRIX".into()))
        );
    }

    // --- FrontendType::normalize ---

    #[test]
    fn normalize_collapses_known_aliases() {
        assert!(matches!(
            FrontendType::Custom("telegram".into()).normalize(),
            FrontendType::Telegram
        ));
        assert!(matches!(
            FrontendType::Custom("Telegram".into()).normalize(),
            FrontendType::Telegram
        ));
        assert!(matches!(
            FrontendType::Custom("DISCORD".into()).normalize(),
            FrontendType::Discord
        ));
        assert!(matches!(
            FrontendType::Custom("whatsapp".into()).normalize(),
            FrontendType::WhatsApp
        ));
        assert!(matches!(
            FrontendType::Custom("whats_app".into()).normalize(),
            FrontendType::WhatsApp
        ));
        assert!(matches!(
            FrontendType::Custom("Whats_App".into()).normalize(),
            FrontendType::WhatsApp
        ));
        assert!(matches!(
            FrontendType::Custom("slack".into()).normalize(),
            FrontendType::Slack
        ));
        assert!(matches!(
            FrontendType::Custom("web".into()).normalize(),
            FrontendType::Web
        ));
        assert!(matches!(
            FrontendType::Custom("cli".into()).normalize(),
            FrontendType::Cli
        ));
    }

    #[test]
    fn normalize_lowercases_and_trims_unknown() {
        let ft = FrontendType::Custom("  MATRIX  ".into()).normalize();
        assert!(matches!(ft, FrontendType::Custom(ref s) if s == "matrix"));
    }

    #[test]
    fn normalize_known_variants_pass_through() {
        assert!(matches!(
            FrontendType::Telegram.normalize(),
            FrontendType::Telegram
        ));
        assert!(matches!(
            FrontendType::Discord.normalize(),
            FrontendType::Discord
        ));
        assert!(matches!(
            FrontendType::WhatsApp.normalize(),
            FrontendType::WhatsApp
        ));
    }

    // --- InMemoryIdentityStore cross-variant normalization ---

    #[tokio::test]
    async fn test_resolve_with_custom_alias() {
        let store = InMemoryIdentityStore::new();

        // Create identity via concrete variant
        let user = store
            .create_identity(FrontendType::Telegram, "123")
            .await
            .unwrap();

        // Resolve via Custom alias — must find the same identity
        let resolved = store
            .resolve(&FrontendType::Custom("telegram".into()), "123")
            .await;
        assert!(
            resolved.is_some(),
            "Custom alias should resolve to the same identity"
        );
        assert_eq!(resolved.unwrap().id, user.id);
    }

    #[tokio::test]
    async fn test_duplicate_detection_cross_variant() {
        let store = InMemoryIdentityStore::new();

        // Create identity via concrete variant
        store
            .create_identity(FrontendType::Telegram, "123")
            .await
            .unwrap();

        // Attempt to create again via Custom alias — must be rejected
        let result = store
            .create_identity(FrontendType::Custom("telegram".into()), "123")
            .await;
        assert!(
            matches!(result, Err(IdentityError::FrontendAlreadyLinked { .. })),
            "duplicate through Custom alias should be rejected"
        );
    }

    #[tokio::test]
    async fn test_remove_link_cross_variant() {
        let store = InMemoryIdentityStore::new();

        // Create identity via concrete variant
        store
            .create_identity(FrontendType::Telegram, "123")
            .await
            .unwrap();

        // Remove via Custom alias — must succeed
        let result = store
            .remove_link(&FrontendType::Custom("telegram".into()), "123")
            .await;
        assert!(
            result.is_ok(),
            "remove_link via Custom alias should succeed"
        );

        // Verify it's actually gone
        let resolved = store.resolve(&FrontendType::Telegram, "123").await;
        assert!(resolved.is_none(), "link should be removed");
    }

    #[tokio::test]
    async fn test_create_link_cross_variant_duplicate_rejected() {
        let store = InMemoryIdentityStore::new();

        // Create identity via concrete variant
        let user = store
            .create_identity(FrontendType::Discord, "456")
            .await
            .unwrap();

        // Attempt to create a link with Custom alias of the same platform + user
        let link = FrontendLink::new(
            user.id,
            FrontendType::Custom("discord".into()),
            "456",
            LinkVerificationMethod::InitialCreation,
            false,
        );
        let result = store.create_link(link).await;
        assert!(
            matches!(result, Err(IdentityError::FrontendAlreadyLinked { .. })),
            "duplicate link through Custom alias should be rejected"
        );
    }
}
