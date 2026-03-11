use chrono::Utc;

use super::error::{IdentityError, IdentityResult};
use super::types::{
    AstridUserId, LinkVerificationMethod, PendingLinkCode, PlatformLink, normalize_platform,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Identity store trait for managing user identities.
///
/// Implementations should handle storage (in-memory, database, etc.)
/// and provide thread-safe access to identity data.
#[async_trait::async_trait]
pub trait IdentityStore: Send + Sync {
    /// Resolve a platform user to their Astrid identity.
    async fn resolve(&self, platform: &str, platform_user_id: &str) -> Option<AstridUserId>;

    /// Get an identity by its Astrid ID.
    async fn get_by_id(&self, id: Uuid) -> Option<AstridUserId>;

    /// Create a new identity for a first-time user.
    async fn create_identity(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> IdentityResult<AstridUserId>;

    /// Create a link between a platform account and an existing identity.
    async fn create_link(&self, link: PlatformLink) -> IdentityResult<()>;

    /// Remove a link between a platform account and an identity.
    async fn remove_link(&self, platform: &str, platform_user_id: &str) -> IdentityResult<()>;

    /// Get all links for an identity.
    async fn get_links(&self, astrid_id: Uuid) -> Vec<PlatformLink>;

    /// Update an identity.
    async fn update_identity(&self, identity: AstridUserId) -> IdentityResult<()>;

    /// Generate a link verification code.
    async fn generate_link_code(
        &self,
        astrid_id: Uuid,
        requesting_platform: &str,
        requesting_user_id: &str,
    ) -> IdentityResult<String>;

    /// Verify a link code and create the link.
    async fn verify_link_code(
        &self,
        code: &str,
        verified_via: &str,
    ) -> IdentityResult<PlatformLink>;
}

/// In-memory identity store for testing and simple deployments.
#[derive(Debug, Default)]
pub struct InMemoryIdentityStore {
    identities: std::sync::RwLock<HashMap<Uuid, AstridUserId>>,
    /// Key: `(normalized_platform, platform_user_id)`
    links: std::sync::RwLock<HashMap<(String, String), PlatformLink>>,
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
    async fn resolve(&self, platform: &str, platform_user_id: &str) -> Option<AstridUserId> {
        let normalized = normalize_platform(platform);
        let links = self.links.read().ok()?;
        let link = links.get(&(normalized, platform_user_id.to_string()))?;
        let identities = self.identities.read().ok()?;
        identities.get(&link.astrid_id).cloned()
    }

    async fn get_by_id(&self, id: Uuid) -> Option<AstridUserId> {
        let identities = self.identities.read().ok()?;
        identities.get(&id).cloned()
    }

    async fn create_identity(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> IdentityResult<AstridUserId> {
        let normalized = normalize_platform(platform);

        // Check if already linked
        {
            let links = self
                .links
                .read()
                .map_err(|e| IdentityError::Internal(format!("Failed to read links: {e}")))?;
            if let Some(existing) = links.get(&(normalized.clone(), platform_user_id.to_string())) {
                return Err(IdentityError::PlatformAlreadyLinked {
                    platform: normalized,
                    existing_id: existing.astrid_id.to_string(),
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
                .map_err(|e| IdentityError::Internal(format!("Failed to write identities: {e}")))?;
            identities.insert(id, identity.clone());
        }

        // Create initial link
        let link = PlatformLink::new(
            id,
            &normalized,
            platform_user_id,
            LinkVerificationMethod::InitialCreation,
            true,
        );

        {
            let mut links = self
                .links
                .write()
                .map_err(|e| IdentityError::Internal(format!("Failed to write links: {e}")))?;
            links.insert((normalized, platform_user_id.to_string()), link);
        }

        Ok(identity)
    }

    async fn create_link(&self, link: PlatformLink) -> IdentityResult<()> {
        let mut links = self
            .links
            .write()
            .map_err(|e| IdentityError::Internal(format!("Failed to write links: {e}")))?;

        let key = (link.platform.clone(), link.platform_user_id.clone());
        if links.contains_key(&key) {
            return Err(IdentityError::PlatformAlreadyLinked {
                platform: link.platform.clone(),
                existing_id: link.astrid_id.to_string(),
            });
        }

        links.insert(key, link);
        Ok(())
    }

    async fn remove_link(&self, platform: &str, platform_user_id: &str) -> IdentityResult<()> {
        let mut links = self
            .links
            .write()
            .map_err(|e| IdentityError::Internal(format!("Failed to write links: {e}")))?;

        let normalized = normalize_platform(platform);
        let key = (normalized, platform_user_id.to_string());
        links.remove(&key).ok_or_else(|| {
            IdentityError::NotFound(format!("No link found for {platform}:{platform_user_id}"))
        })?;

        Ok(())
    }

    async fn get_links(&self, astrid_id: Uuid) -> Vec<PlatformLink> {
        let Ok(links) = self.links.read() else {
            return Vec::new();
        };

        links
            .values()
            .filter(|link| link.astrid_id == astrid_id)
            .cloned()
            .collect()
    }

    async fn update_identity(&self, identity: AstridUserId) -> IdentityResult<()> {
        let mut identities = self
            .identities
            .write()
            .map_err(|e| IdentityError::Internal(format!("Failed to write identities: {e}")))?;

        if !identities.contains_key(&identity.id) {
            return Err(IdentityError::NotFound(identity.id.to_string()));
        }

        identities.insert(identity.id, identity);
        Ok(())
    }

    async fn generate_link_code(
        &self,
        astrid_id: Uuid,
        requesting_platform: &str,
        requesting_user_id: &str,
    ) -> IdentityResult<String> {
        let code = Self::generate_code();

        let pending = PendingLinkCode {
            code: code.clone(),
            astrid_id,
            requesting_platform: normalize_platform(requesting_platform),
            requesting_user_id: requesting_user_id.to_string(),
            // Safety: chrono::Duration addition to DateTime cannot overflow for reasonable durations
            #[expect(clippy::arithmetic_side_effects)]
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };

        let mut codes = self
            .pending_codes
            .write()
            .map_err(|e| IdentityError::Internal(format!("Failed to write pending codes: {e}")))?;
        codes.insert(code.clone(), pending);

        Ok(code)
    }

    async fn verify_link_code(
        &self,
        code: &str,
        verified_via: &str,
    ) -> IdentityResult<PlatformLink> {
        // Get and remove the pending code
        let pending = {
            let mut codes = self.pending_codes.write().map_err(|e| {
                IdentityError::Internal(format!("Failed to write pending codes: {e}"))
            })?;
            codes.remove(code).ok_or(IdentityError::VerificationFailed(
                "Invalid or expired code".to_string(),
            ))?
        };

        if pending.is_expired() {
            return Err(IdentityError::VerificationExpired);
        }

        // Create the link
        let link = PlatformLink::new(
            pending.astrid_id,
            &pending.requesting_platform,
            &pending.requesting_user_id,
            LinkVerificationMethod::CodeVerification {
                verified_via: normalize_platform(verified_via),
            },
            false,
        );

        self.create_link(link.clone()).await?;

        Ok(link)
    }
}
