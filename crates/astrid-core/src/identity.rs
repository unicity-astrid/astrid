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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{SecurityError, SecurityResult};

/// Astrid-native user identity (spans all frontends).
///
/// This is the canonical identifier for a user across all platforms.
/// The same `AstridUserId` is used whether the user is on Discord,
/// WhatsApp, Telegram, or any other frontend.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AstridUserId {
    /// Unique identifier (UUID)
    pub id: Uuid,
    /// Optional ed25519 public key for signing (32 bytes)
    #[serde(
        serialize_with = "serialize_optional_key",
        deserialize_with = "deserialize_optional_key"
    )]
    pub public_key: Option<[u8; 32]>,
    /// Display name
    pub display_name: Option<String>,
    /// When created
    pub created_at: DateTime<Utc>,
}

// Serde's serialize_with requires &Option<T> signature, not Option<&T>
#[allow(clippy::ref_option)]
fn serialize_optional_key<S>(key: &Option<[u8; 32]>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match key {
        Some(bytes) => {
            let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
            serializer.serialize_some(&encoded)
        },
        None => serializer.serialize_none(),
    }
}

fn deserialize_optional_key<'de, D>(deserializer: D) -> Result<Option<[u8; 32]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => {
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &s)
                .map_err(serde::de::Error::custom)?;
            if bytes.len() != 32 {
                return Err(serde::de::Error::custom("public key must be 32 bytes"));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Some(arr))
        },
        None => Ok(None),
    }
}

impl AstridUserId {
    /// Create a new Astrid user identity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            public_key: None,
            display_name: None,
            created_at: Utc::now(),
        }
    }

    /// Create an identity with a specific UUID.
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self {
            id,
            public_key: None,
            display_name: None,
            created_at: Utc::now(),
        }
    }

    /// Create an identity with a display name.
    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Set the ed25519 public key for this identity.
    #[must_use]
    pub fn with_public_key(mut self, key: [u8; 32]) -> Self {
        self.public_key = Some(key);
        self
    }

    /// Check if this identity has a registered signing key.
    #[must_use]
    pub fn has_signing_key(&self) -> bool {
        self.public_key.is_some()
    }
}

impl Default for AstridUserId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AstridUserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref name) = self.display_name {
            write!(f, "{}({})", name, &self.id.to_string()[..8])
        } else {
            write!(f, "user:{}", &self.id.to_string()[..8])
        }
    }
}

/// Links a frontend account to an Astrid identity.
///
/// This enables cross-frontend identity - the same user on Discord
/// and WhatsApp will have the same `AstridUserId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontendLink {
    /// The Astrid identity this frontend account is linked to
    pub astrid_id: Uuid,
    /// Which frontend platform
    pub frontend: FrontendType,
    /// Platform-specific user ID (e.g., Discord snowflake, phone number)
    pub frontend_user_id: String,
    /// When this link was created
    pub linked_at: DateTime<Utc>,
    /// How this link was verified
    pub verification_method: LinkVerificationMethod,
    /// Whether this is the primary (first linked) frontend
    pub is_primary: bool,
}

impl FrontendLink {
    /// Create a new frontend link.
    #[must_use]
    pub fn new(
        astrid_id: Uuid,
        frontend: FrontendType,
        frontend_user_id: impl Into<String>,
        verification_method: LinkVerificationMethod,
        is_primary: bool,
    ) -> Self {
        Self {
            astrid_id,
            frontend,
            frontend_user_id: frontend_user_id.into(),
            linked_at: Utc::now(),
            verification_method,
            is_primary,
        }
    }
}

/// Supported frontend platforms.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontendType {
    /// Discord bot integration
    Discord,
    /// WhatsApp integration
    WhatsApp,
    /// Telegram bot integration
    Telegram,
    /// Slack integration
    Slack,
    /// Web dashboard
    Web,
    /// Command-line interface
    Cli,
    /// Custom/third-party frontend
    Custom(String),
}

impl FrontendType {
    /// Return a lowercase canonical key for this platform.
    ///
    /// Known variants return their static name; [`Custom`](Self::Custom) values
    /// are lowercased and, if they match a known variant, collapse to that
    /// variant's key (e.g. `Custom("Telegram")` → `"telegram"`). Unknown
    /// platforms are trimmed and lowercased.
    ///
    /// Returns an empty `Cow::Owned("")` for [`Custom`](Self::Custom) values
    /// that are empty or whitespace-only after trimming. Callers constructing
    /// `Custom` directly should validate the inner string is non-empty.
    ///
    /// This should be used instead of raw `PartialEq` when comparing platform
    /// identity across trust boundaries (e.g. WASM guests, MCP plugins).
    #[must_use]
    pub fn canonical_name(&self) -> Cow<'_, str> {
        match self {
            Self::Discord => Cow::Borrowed("discord"),
            Self::WhatsApp => Cow::Borrowed("whatsapp"),
            Self::Telegram => Cow::Borrowed("telegram"),
            Self::Slack => Cow::Borrowed("slack"),
            Self::Web => Cow::Borrowed("web"),
            Self::Cli => Cow::Borrowed("cli"),
            Self::Custom(name) => {
                // Collapse known aliases back to their canonical form.
                let trimmed = name.trim();
                match trimmed {
                    _ if trimmed.eq_ignore_ascii_case("discord") => Cow::Borrowed("discord"),
                    _ if trimmed.eq_ignore_ascii_case("whatsapp") => Cow::Borrowed("whatsapp"),
                    _ if trimmed.eq_ignore_ascii_case("whats_app") => Cow::Borrowed("whatsapp"),
                    _ if trimmed.eq_ignore_ascii_case("telegram") => Cow::Borrowed("telegram"),
                    _ if trimmed.eq_ignore_ascii_case("slack") => Cow::Borrowed("slack"),
                    _ if trimmed.eq_ignore_ascii_case("web") => Cow::Borrowed("web"),
                    _ if trimmed.eq_ignore_ascii_case("cli") => Cow::Borrowed("cli"),
                    _ => Cow::Owned(trimmed.to_ascii_lowercase()),
                }
            },
        }
    }

    /// Returns `true` if `self` and `other` refer to the same logical platform,
    /// normalizing [`Custom`](Self::Custom) aliases and ignoring case.
    ///
    /// # Examples
    ///
    /// ```
    /// use astrid_core::identity::FrontendType;
    ///
    /// assert!(FrontendType::Telegram.is_same_platform(&FrontendType::Custom("telegram".into())));
    /// assert!(FrontendType::Telegram.is_same_platform(&FrontendType::Custom("Telegram".into())));
    /// assert!(FrontendType::Custom("MATRIX".into()).is_same_platform(&FrontendType::Custom("matrix".into())));
    /// assert!(!FrontendType::Discord.is_same_platform(&FrontendType::Telegram));
    /// ```
    #[must_use]
    pub fn is_same_platform(&self, other: &Self) -> bool {
        self.canonical_name() == other.canonical_name()
    }
}

impl fmt::Display for FrontendType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Discord => write!(f, "discord"),
            Self::WhatsApp => write!(f, "whatsapp"),
            Self::Telegram => write!(f, "telegram"),
            Self::Slack => write!(f, "slack"),
            Self::Web => write!(f, "web"),
            Self::Cli => write!(f, "cli"),
            Self::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

/// How a frontend link was verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkVerificationMethod {
    /// First frontend creates identity (bootstrap)
    InitialCreation,
    /// Code sent to verified frontend, entered in new frontend
    CodeVerification {
        /// The frontend that verified the code
        verified_via: FrontendType,
    },
    /// Admin manually linked the accounts
    AdminLink {
        /// Admin who performed the link
        admin_id: Uuid,
    },
}

impl fmt::Display for LinkVerificationMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitialCreation => write!(f, "initial_creation"),
            Self::CodeVerification { verified_via } => write!(f, "code_via:{verified_via}"),
            Self::AdminLink { admin_id } => write!(f, "admin:{}", &admin_id.to_string()[..8]),
        }
    }
}

/// Pending link verification code.
#[derive(Debug, Clone)]
pub struct PendingLinkCode {
    /// The verification code
    pub code: String,
    /// The Astrid identity being linked
    pub astrid_id: Uuid,
    /// The frontend requesting the link
    pub requesting_frontend: FrontendType,
    /// The frontend user ID on the requesting frontend
    pub requesting_user_id: String,
    /// When this code expires
    pub expires_at: DateTime<Utc>,
}

impl PendingLinkCode {
    /// Check if this code has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// Identity store trait for managing user identities.
///
/// Implementations should handle storage (in-memory, database, etc.)
/// and provide thread-safe access to identity data.
#[async_trait::async_trait]
pub trait IdentityStore: Send + Sync {
    /// Resolve a frontend user to their Astrid identity.
    async fn resolve(
        &self,
        frontend: &FrontendType,
        frontend_user_id: &str,
    ) -> Option<AstridUserId>;

    /// Get an identity by its Astrid ID.
    async fn get_by_id(&self, id: Uuid) -> Option<AstridUserId>;

    /// Create a new identity for a first-time user.
    async fn create_identity(
        &self,
        frontend: FrontendType,
        frontend_user_id: &str,
    ) -> SecurityResult<AstridUserId>;

    /// Create a link between a frontend account and an existing identity.
    async fn create_link(&self, link: FrontendLink) -> SecurityResult<()>;

    /// Remove a link between a frontend account and an identity.
    async fn remove_link(
        &self,
        frontend: &FrontendType,
        frontend_user_id: &str,
    ) -> SecurityResult<()>;

    /// Get all links for an identity.
    async fn get_links(&self, astrid_id: Uuid) -> Vec<FrontendLink>;

    /// Update an identity.
    async fn update_identity(&self, identity: AstridUserId) -> SecurityResult<()>;

    /// Generate a link verification code.
    async fn generate_link_code(
        &self,
        astrid_id: Uuid,
        requesting_frontend: FrontendType,
        requesting_user_id: &str,
    ) -> SecurityResult<String>;

    /// Verify a link code and create the link.
    async fn verify_link_code(
        &self,
        code: &str,
        verified_via: FrontendType,
    ) -> SecurityResult<FrontendLink>;
}

/// In-memory identity store for testing and simple deployments.
#[derive(Debug, Default)]
pub struct InMemoryIdentityStore {
    identities: std::sync::RwLock<HashMap<Uuid, AstridUserId>>,
    links: std::sync::RwLock<HashMap<(FrontendType, String), FrontendLink>>,
    pending_codes: std::sync::RwLock<HashMap<String, PendingLinkCode>>,
}

impl InMemoryIdentityStore {
    /// Create a new in-memory identity store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap in an Arc for sharing.
    #[must_use]
    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    fn generate_code() -> String {
        use rand::Rng;
        let code: u32 = rand::rngs::OsRng.gen_range(0..1_000_000_000);
        format!("{code:09}")
    }
}

#[async_trait::async_trait]
impl IdentityStore for InMemoryIdentityStore {
    async fn resolve(
        &self,
        frontend: &FrontendType,
        frontend_user_id: &str,
    ) -> Option<AstridUserId> {
        let links = self.links.read().ok()?;
        let link = links.get(&(frontend.clone(), frontend_user_id.to_string()))?;
        let identities = self.identities.read().ok()?;
        identities.get(&link.astrid_id).cloned()
    }

    async fn get_by_id(&self, id: Uuid) -> Option<AstridUserId> {
        let identities = self.identities.read().ok()?;
        identities.get(&id).cloned()
    }

    async fn create_identity(
        &self,
        frontend: FrontendType,
        frontend_user_id: &str,
    ) -> SecurityResult<AstridUserId> {
        // Check if already linked
        {
            let links = self
                .links
                .read()
                .map_err(|e| SecurityError::Internal(format!("Failed to read links: {e}")))?;
            if links.contains_key(&(frontend.clone(), frontend_user_id.to_string())) {
                return Err(SecurityError::FrontendAlreadyLinked {
                    frontend: frontend.to_string(),
                    existing_id: "unknown".to_string(),
                });
            }
        }

        let identity = AstridUserId::new();
        let id = identity.id;

        // Store identity
        {
            let mut identities = self
                .identities
                .write()
                .map_err(|e| SecurityError::Internal(format!("Failed to write identities: {e}")))?;
            identities.insert(id, identity.clone());
        }

        // Create initial link
        let link = FrontendLink::new(
            id,
            frontend.clone(),
            frontend_user_id,
            LinkVerificationMethod::InitialCreation,
            true,
        );

        {
            let mut links = self
                .links
                .write()
                .map_err(|e| SecurityError::Internal(format!("Failed to write links: {e}")))?;
            links.insert((frontend, frontend_user_id.to_string()), link);
        }

        Ok(identity)
    }

    async fn create_link(&self, link: FrontendLink) -> SecurityResult<()> {
        let mut links = self
            .links
            .write()
            .map_err(|e| SecurityError::Internal(format!("Failed to write links: {e}")))?;

        let key = (link.frontend.clone(), link.frontend_user_id.clone());
        if links.contains_key(&key) {
            return Err(SecurityError::FrontendAlreadyLinked {
                frontend: link.frontend.to_string(),
                existing_id: link.astrid_id.to_string(),
            });
        }

        links.insert(key, link);
        Ok(())
    }

    async fn remove_link(
        &self,
        frontend: &FrontendType,
        frontend_user_id: &str,
    ) -> SecurityResult<()> {
        let mut links = self
            .links
            .write()
            .map_err(|e| SecurityError::Internal(format!("Failed to write links: {e}")))?;

        let key = (frontend.clone(), frontend_user_id.to_string());
        links.remove(&key).ok_or_else(|| {
            SecurityError::IdentityNotFound(format!(
                "No link found for {frontend}:{frontend_user_id}"
            ))
        })?;

        Ok(())
    }

    async fn get_links(&self, astrid_id: Uuid) -> Vec<FrontendLink> {
        let Ok(links) = self.links.read() else {
            return Vec::new();
        };

        links
            .values()
            .filter(|link| link.astrid_id == astrid_id)
            .cloned()
            .collect()
    }

    async fn update_identity(&self, identity: AstridUserId) -> SecurityResult<()> {
        let mut identities = self
            .identities
            .write()
            .map_err(|e| SecurityError::Internal(format!("Failed to write identities: {e}")))?;

        if !identities.contains_key(&identity.id) {
            return Err(SecurityError::IdentityNotFound(identity.id.to_string()));
        }

        identities.insert(identity.id, identity);
        Ok(())
    }

    async fn generate_link_code(
        &self,
        astrid_id: Uuid,
        requesting_frontend: FrontendType,
        requesting_user_id: &str,
    ) -> SecurityResult<String> {
        let code = Self::generate_code();

        let pending = PendingLinkCode {
            code: code.clone(),
            astrid_id,
            requesting_frontend,
            requesting_user_id: requesting_user_id.to_string(),
            // Safety: chrono::Duration addition to DateTime cannot overflow for reasonable durations
            #[allow(clippy::arithmetic_side_effects)]
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };

        let mut codes = self
            .pending_codes
            .write()
            .map_err(|e| SecurityError::Internal(format!("Failed to write pending codes: {e}")))?;
        codes.insert(code.clone(), pending);

        Ok(code)
    }

    async fn verify_link_code(
        &self,
        code: &str,
        verified_via: FrontendType,
    ) -> SecurityResult<FrontendLink> {
        // Get and remove the pending code
        let pending = {
            let mut codes = self.pending_codes.write().map_err(|e| {
                SecurityError::Internal(format!("Failed to write pending codes: {e}"))
            })?;
            codes
                .remove(code)
                .ok_or(SecurityError::IdentityVerificationFailed(
                    "Invalid or expired code".to_string(),
                ))?
        };

        if pending.is_expired() {
            return Err(SecurityError::VerificationExpired);
        }

        // Create the link
        let link = FrontendLink::new(
            pending.astrid_id,
            pending.requesting_frontend,
            &pending.requesting_user_id,
            LinkVerificationMethod::CodeVerification { verified_via },
            false,
        );

        self.create_link(link.clone()).await?;

        Ok(link)
    }
}

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
        assert!(FrontendType::Custom("".into()).is_same_platform(&FrontendType::Custom("".into())));
        assert!(
            FrontendType::Custom("".into()).is_same_platform(&FrontendType::Custom("   ".into()))
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
            Err(SecurityError::FrontendAlreadyLinked { .. })
        ));
    }
}
