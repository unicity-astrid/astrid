//! Capability tokens - cryptographically signed authorization.
//!
//! A capability token grants specific permissions to access resources.
//! Tokens are:
//! - Signed by the runtime's ed25519 key
//! - Linked to the approval event that created them
//! - Scoped (session or persistent)
//! - Time-bounded (optional expiration)

use astrid_core::{Permission, Timestamp, TokenId};
use astrid_crypto::{ContentHash, KeyPair, PublicKey, Signature};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CapabilityError, CapabilityResult};
use crate::pattern::ResourcePattern;

/// Version of the signing data format.
/// Increment this when the signing data structure changes.
const SIGNING_DATA_VERSION: u8 = 0x01;

/// Default clock skew tolerance in seconds.
const DEFAULT_CLOCK_SKEW_SECS: i64 = 30;

/// Write a length-prefixed byte slice to the output buffer.
///
/// Format: 4-byte little-endian length followed by the data.
#[allow(clippy::cast_possible_truncation)]
fn write_length_prefixed(data: &mut Vec<u8>, bytes: &[u8]) {
    // Length is limited to u32::MAX; larger slices would be truncated.
    // This is acceptable as capability token fields are small.
    data.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    data.extend_from_slice(bytes);
}

/// Unique identifier for an audit entry (used for linking).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuditEntryId(pub Uuid);

impl AuditEntryId {
    /// Create a new audit entry ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AuditEntryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AuditEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "audit:{}", &self.0.to_string()[..8])
    }
}

/// Token scope - how long it lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenScope {
    /// Valid only for the current session (in-memory).
    Session,
    /// Persisted across sessions.
    Persistent,
}

impl std::fmt::Display for TokenScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session => write!(f, "session"),
            Self::Persistent => write!(f, "persistent"),
        }
    }
}

/// A capability token granting permissions for a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityToken {
    /// Unique token identifier.
    pub id: TokenId,
    /// Resource pattern this token applies to.
    pub resource: ResourcePattern,
    /// Permissions granted.
    pub permissions: Vec<Permission>,
    /// When the token was issued.
    pub issued_at: Timestamp,
    /// When the token expires (None = no expiration within scope).
    pub expires_at: Option<Timestamp>,
    /// Token scope (session or persistent).
    pub scope: TokenScope,
    /// Public key of the issuer (runtime).
    pub issuer: PublicKey,
    /// User who approved this token (key ID, first 8 bytes).
    pub user_id: [u8; 8],
    /// Audit entry ID linking to the approval event.
    pub approval_audit_id: AuditEntryId,
    /// Whether this token can only be used once (replay protection).
    #[serde(default)]
    pub single_use: bool,
    /// Cryptographic signature of the token.
    pub signature: Signature,
}

impl CapabilityToken {
    /// Create a new capability token.
    ///
    /// This is typically called by the runtime after user approval.
    #[must_use]
    pub fn create(
        resource: ResourcePattern,
        permissions: Vec<Permission>,
        scope: TokenScope,
        user_id: [u8; 8],
        approval_audit_id: AuditEntryId,
        runtime_key: &KeyPair,
        ttl: Option<Duration>,
    ) -> Self {
        Self::create_with_options(
            resource,
            permissions,
            scope,
            user_id,
            approval_audit_id,
            runtime_key,
            ttl,
            false,
        )
    }

    /// Create a new capability token with additional options.
    ///
    /// This is typically called by the runtime after user approval.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn create_with_options(
        resource: ResourcePattern,
        permissions: Vec<Permission>,
        scope: TokenScope,
        user_id: [u8; 8],
        approval_audit_id: AuditEntryId,
        runtime_key: &KeyPair,
        ttl: Option<Duration>,
        single_use: bool,
    ) -> Self {
        let id = TokenId::new();
        let issued_at = Timestamp::now();
        let expires_at = ttl.map(|d| {
            // Safety: chrono Duration addition to DateTime cannot overflow for reasonable durations
            #[allow(clippy::arithmetic_side_effects)]
            let expiry = Utc::now() + d;
            Timestamp::from_datetime(expiry)
        });
        let issuer = runtime_key.export_public_key();

        // Create token without signature for signing
        let mut token = Self {
            id,
            resource,
            permissions,
            issued_at,
            expires_at,
            scope,
            issuer,
            user_id,
            approval_audit_id,
            single_use,
            signature: Signature::from_bytes([0u8; 64]), // Placeholder
        };

        // Sign the token
        let signing_data = token.signing_data();
        token.signature = runtime_key.sign(&signing_data);

        token
    }

    /// Get the data used for signing (excludes the signature itself).
    ///
    /// Format (v1):
    /// - 1 byte: version (0x01)
    /// - Length-prefixed token ID (UUID bytes)
    /// - Length-prefixed resource pattern string
    /// - 4 bytes: number of permissions
    /// - For each permission: length-prefixed string
    /// - 8 bytes: `issued_at` timestamp (i64 LE)
    /// - 1 byte: `has_expiration` flag
    /// - If `has_expiration`: 8 bytes expiration timestamp (i64 LE)
    /// - Length-prefixed scope string
    /// - 32 bytes: issuer public key
    /// - 8 bytes: `user_id`
    /// - Length-prefixed audit entry ID (UUID bytes)
    /// - 1 byte: `single_use` flag
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn signing_data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(512);

        // Version prefix
        data.push(SIGNING_DATA_VERSION);

        // Token ID
        write_length_prefixed(&mut data, self.id.0.as_bytes());

        // Resource pattern
        write_length_prefixed(&mut data, self.resource.as_str().as_bytes());

        // Permissions count and values
        data.extend_from_slice(&(self.permissions.len() as u32).to_le_bytes());
        for perm in &self.permissions {
            write_length_prefixed(&mut data, perm.to_string().as_bytes());
        }

        // Issued at
        data.extend_from_slice(&self.issued_at.0.timestamp().to_le_bytes());

        // Expiration (with presence flag)
        if let Some(expires) = &self.expires_at {
            data.push(0x01); // has expiration
            data.extend_from_slice(&expires.0.timestamp().to_le_bytes());
        } else {
            data.push(0x00); // no expiration
        }

        // Scope
        write_length_prefixed(&mut data, self.scope.to_string().as_bytes());

        // Issuer (fixed 32 bytes)
        data.extend_from_slice(self.issuer.as_bytes());

        // User ID (fixed 8 bytes)
        data.extend_from_slice(&self.user_id);

        // Approval audit ID
        write_length_prefixed(&mut data, self.approval_audit_id.0.as_bytes());

        // Single use flag
        data.push(u8::from(self.single_use));

        data
    }

    /// Verify the token's signature.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::InvalidSignature`] if the signature is invalid.
    pub fn verify_signature(&self) -> CapabilityResult<()> {
        let signing_data = self.signing_data();
        self.issuer
            .verify(&signing_data, &self.signature)
            .map_err(|_| CapabilityError::InvalidSignature)
    }

    /// Check if the token has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.is_expired_with_skew(0)
    }

    /// Check if the token has expired, with clock skew tolerance.
    ///
    /// A positive `skew_secs` value allows tokens that expired up to
    /// that many seconds ago to still be considered valid.
    #[must_use]
    pub fn is_expired_with_skew(&self, skew_secs: i64) -> bool {
        self.expires_at.as_ref().is_some_and(|exp| {
            let now = Utc::now();
            // Safety: chrono Duration addition to DateTime cannot overflow for reasonable skew values
            #[allow(clippy::arithmetic_side_effects)]
            let adjusted_expiry = exp.0 + Duration::seconds(skew_secs);
            now > adjusted_expiry
        })
    }

    /// Check if the token is valid (not expired, signature OK).
    ///
    /// Uses the default clock skew tolerance (30 seconds).
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::TokenExpired`] if expired,
    /// or [`CapabilityError::InvalidSignature`] if the signature is invalid.
    pub fn validate(&self) -> CapabilityResult<()> {
        self.validate_with_skew(DEFAULT_CLOCK_SKEW_SECS)
    }

    /// Check if the token is valid with custom clock skew tolerance.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::TokenExpired`] if expired,
    /// or [`CapabilityError::InvalidSignature`] if the signature is invalid.
    pub fn validate_with_skew(&self, skew_secs: i64) -> CapabilityResult<()> {
        if self.is_expired_with_skew(skew_secs) {
            return Err(CapabilityError::TokenExpired {
                token_id: self.id.to_string(),
            });
        }
        self.verify_signature()
    }

    /// Check if this is a single-use token.
    #[must_use]
    pub fn is_single_use(&self) -> bool {
        self.single_use
    }

    /// Check if this token grants a permission for a resource.
    #[must_use]
    pub fn grants(&self, resource: &str, permission: Permission) -> bool {
        self.resource.matches(resource) && self.permissions.contains(&permission)
    }

    /// Hash the token for audit purposes.
    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        ContentHash::hash(&self.signing_data())
    }
}

impl PartialEq for CapabilityToken {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for CapabilityToken {}

impl std::hash::Hash for CapabilityToken {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Builder for creating capability tokens with fluent API.
pub struct TokenBuilder {
    resource: ResourcePattern,
    permissions: Vec<Permission>,
    scope: TokenScope,
    ttl: Option<Duration>,
    single_use: bool,
}

impl TokenBuilder {
    /// Create a new token builder.
    #[must_use]
    pub fn new(resource: ResourcePattern) -> Self {
        Self {
            resource,
            permissions: Vec::new(),
            scope: TokenScope::Session,
            ttl: None,
            single_use: false,
        }
    }

    /// Add a permission.
    #[must_use]
    pub fn permission(mut self, perm: Permission) -> Self {
        if !self.permissions.contains(&perm) {
            self.permissions.push(perm);
        }
        self
    }

    /// Add multiple permissions.
    #[must_use]
    pub fn permissions(mut self, perms: impl IntoIterator<Item = Permission>) -> Self {
        for perm in perms {
            if !self.permissions.contains(&perm) {
                self.permissions.push(perm);
            }
        }
        self
    }

    /// Set the scope.
    #[must_use]
    pub fn scope(mut self, scope: TokenScope) -> Self {
        self.scope = scope;
        self
    }

    /// Set persistent scope.
    #[must_use]
    pub fn persistent(self) -> Self {
        self.scope(TokenScope::Persistent)
    }

    /// Set session scope.
    #[must_use]
    pub fn session(self) -> Self {
        self.scope(TokenScope::Session)
    }

    /// Set time-to-live.
    #[must_use]
    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Mark token as single-use (for replay protection).
    #[must_use]
    pub fn single_use(mut self) -> Self {
        self.single_use = true;
        self
    }

    /// Build the token (requires runtime key and user context).
    #[must_use]
    pub fn build(
        self,
        user_id: [u8; 8],
        approval_audit_id: AuditEntryId,
        runtime_key: &KeyPair,
    ) -> CapabilityToken {
        CapabilityToken::create_with_options(
            self.resource,
            self.permissions,
            self.scope,
            user_id,
            approval_audit_id,
            runtime_key,
            self.ttl,
            self.single_use,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrid_core::Permission;

    fn test_keypair() -> KeyPair {
        KeyPair::generate()
    }

    #[test]
    fn test_token_creation() {
        let keypair = test_keypair();
        let pattern = ResourcePattern::exact("mcp://filesystem:read_file").unwrap();

        let token = CapabilityToken::create(
            pattern,
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        assert!(!token.is_expired());
        assert!(token.verify_signature().is_ok());
    }

    #[test]
    fn test_token_grants() {
        let keypair = test_keypair();
        let pattern = ResourcePattern::new("mcp://filesystem:*").unwrap();

        let token = CapabilityToken::create(
            pattern,
            vec![Permission::Invoke, Permission::Read],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        assert!(token.grants("mcp://filesystem:read_file", Permission::Invoke));
        assert!(token.grants("mcp://filesystem:write_file", Permission::Invoke));
        assert!(!token.grants("mcp://filesystem:read_file", Permission::Write));
        assert!(!token.grants("mcp://memory:read", Permission::Invoke));
    }

    #[test]
    fn test_token_expiration() {
        let keypair = test_keypair();
        let pattern = ResourcePattern::exact("test://resource").unwrap();

        // Create expired token (beyond clock skew tolerance of 30s)
        let token = CapabilityToken::create(
            pattern,
            vec![Permission::Read],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            Some(Duration::seconds(-60)), // Expired well beyond skew tolerance
        );

        assert!(token.is_expired());
        assert!(matches!(
            token.validate(),
            Err(CapabilityError::TokenExpired { .. })
        ));
    }

    #[test]
    fn test_token_expiration_with_clock_skew() {
        let keypair = test_keypair();
        let pattern = ResourcePattern::exact("test://resource").unwrap();

        // Create token that just expired but is within clock skew tolerance
        let token = CapabilityToken::create(
            pattern,
            vec![Permission::Read],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            Some(Duration::seconds(-10)), // Just expired
        );

        // Without skew tolerance, it's expired
        assert!(token.is_expired());
        assert!(token.is_expired_with_skew(0));

        // With default 30s skew tolerance, it's still valid
        assert!(!token.is_expired_with_skew(30));
        assert!(token.validate().is_ok());
    }

    #[test]
    fn test_token_builder() {
        let keypair = test_keypair();

        let token =
            TokenBuilder::new(ResourcePattern::exact("mcp://filesystem:read_file").unwrap())
                .permission(Permission::Invoke)
                .permission(Permission::Read)
                .persistent()
                .ttl(Duration::hours(24))
                .build(keypair.key_id(), AuditEntryId::new(), &keypair);

        assert_eq!(token.scope, TokenScope::Persistent);
        assert!(token.expires_at.is_some());
        assert!(token.permissions.contains(&Permission::Invoke));
        assert!(token.permissions.contains(&Permission::Read));
    }

    #[test]
    fn test_token_signature_verification() {
        let keypair = test_keypair();
        let pattern = ResourcePattern::exact("test://resource").unwrap();

        let mut token = CapabilityToken::create(
            pattern,
            vec![Permission::Read],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        // Valid signature
        assert!(token.verify_signature().is_ok());

        // Tamper with token
        token.permissions.push(Permission::Write);

        // Signature should now fail
        assert!(matches!(
            token.verify_signature(),
            Err(CapabilityError::InvalidSignature)
        ));
    }

    #[test]
    fn test_token_content_hash() {
        let keypair = test_keypair();
        let pattern = ResourcePattern::exact("test://resource").unwrap();

        let token = CapabilityToken::create(
            pattern.clone(),
            vec![Permission::Read],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        let hash = token.content_hash();
        assert!(!hash.is_zero());

        // Different token should have different hash
        let token2 = CapabilityToken::create(
            pattern,
            vec![Permission::Write],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        assert_ne!(token.content_hash(), token2.content_hash());
    }
}
