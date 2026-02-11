//! Capability validation logic.
//!
//! Validates tokens and checks authorization for operations.

use astralis_core::Permission;
use astralis_crypto::PublicKey;

use crate::error::{CapabilityError, CapabilityResult};
use crate::store::CapabilityStore;
use crate::token::CapabilityToken;

/// Authorization result after validation.
#[derive(Debug, Clone)]
pub enum AuthorizationResult {
    /// Authorized by a specific token.
    Authorized {
        /// The token that granted authorization.
        token: Box<CapabilityToken>,
    },
    /// Not authorized - approval required.
    RequiresApproval {
        /// The resource being accessed.
        resource: String,
        /// The required permission.
        permission: Permission,
    },
}

impl AuthorizationResult {
    /// Check if authorized.
    #[must_use]
    pub fn is_authorized(&self) -> bool {
        matches!(self, Self::Authorized { .. })
    }

    /// Get the authorizing token if authorized.
    #[must_use]
    pub fn token(&self) -> Option<&CapabilityToken> {
        match self {
            Self::Authorized { token } => Some(token),
            Self::RequiresApproval { .. } => None,
        }
    }
}

/// Capability validator for checking authorization.
pub struct CapabilityValidator<'a> {
    store: &'a CapabilityStore,
    trusted_issuers: Vec<PublicKey>,
}

impl<'a> CapabilityValidator<'a> {
    /// Create a new validator with a capability store.
    #[must_use]
    pub fn new(store: &'a CapabilityStore) -> Self {
        Self {
            store,
            trusted_issuers: Vec::new(),
        }
    }

    /// Add a trusted issuer (runtime public key).
    #[must_use]
    pub fn trust_issuer(mut self, issuer: PublicKey) -> Self {
        self.trusted_issuers.push(issuer);
        self
    }

    /// Check authorization for a resource and permission.
    #[must_use]
    pub fn check(&self, resource: &str, permission: Permission) -> AuthorizationResult {
        if let Some(token) = self.store.find_capability(resource, permission) {
            // Validate the token
            if self.validate_token(&token).is_ok() {
                return AuthorizationResult::Authorized {
                    token: Box::new(token),
                };
            }
        }

        AuthorizationResult::RequiresApproval {
            resource: resource.to_string(),
            permission,
        }
    }

    /// Validate a specific token.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is expired, has an invalid signature,
    /// or is not from a trusted issuer.
    pub fn validate_token(&self, token: &CapabilityToken) -> CapabilityResult<()> {
        // Check expiration
        if token.is_expired() {
            return Err(CapabilityError::TokenExpired {
                token_id: token.id.to_string(),
            });
        }

        // Verify signature
        token.verify_signature()?;

        // Check issuer trust (if we have a list)
        if !self.trusted_issuers.is_empty() && !self.trusted_issuers.contains(&token.issuer) {
            return Err(CapabilityError::InvalidSignature);
        }

        Ok(())
    }

    /// Validate a token by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not found, revoked, or invalid.
    pub fn validate_by_id(&self, token_id: &astralis_core::TokenId) -> CapabilityResult<()> {
        let token = self
            .store
            .get(token_id)?
            .ok_or_else(|| CapabilityError::TokenNotFound {
                token_id: token_id.to_string(),
            })?;

        self.validate_token(&token)
    }
}

/// Check multiple permissions at once.
pub struct MultiPermissionCheck {
    checks: Vec<(String, Permission)>,
}

impl MultiPermissionCheck {
    /// Create a new multi-permission check.
    #[must_use]
    pub fn new() -> Self {
        Self { checks: Vec::new() }
    }

    /// Add a permission check.
    #[must_use]
    pub fn add(mut self, resource: impl Into<String>, permission: Permission) -> Self {
        self.checks.push((resource.into(), permission));
        self
    }

    /// Run all checks against a validator.
    #[must_use]
    pub fn check_all(
        &self,
        validator: &CapabilityValidator<'_>,
    ) -> Vec<(String, Permission, AuthorizationResult)> {
        self.checks
            .iter()
            .map(|(resource, permission)| {
                let result = validator.check(resource, *permission);
                (resource.clone(), *permission, result)
            })
            .collect()
    }

    /// Check if all permissions are authorized.
    #[must_use]
    pub fn all_authorized(&self, validator: &CapabilityValidator<'_>) -> bool {
        self.checks
            .iter()
            .all(|(resource, permission)| validator.check(resource, *permission).is_authorized())
    }

    /// Get permissions that require approval.
    #[must_use]
    pub fn needs_approval(&self, validator: &CapabilityValidator<'_>) -> Vec<(String, Permission)> {
        self.checks
            .iter()
            .filter(|(resource, permission)| {
                !validator.check(resource, *permission).is_authorized()
            })
            .cloned()
            .collect()
    }
}

impl Default for MultiPermissionCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::ResourcePattern;
    use crate::token::{AuditEntryId, TokenScope};
    use astralis_crypto::KeyPair;

    fn test_keypair() -> KeyPair {
        KeyPair::generate()
    }

    #[test]
    fn test_authorization_check() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        store.add(token).unwrap();

        let validator = CapabilityValidator::new(&store);

        let result = validator.check("mcp://test:tool", Permission::Invoke);
        assert!(result.is_authorized());

        let result = validator.check("mcp://test:other", Permission::Invoke);
        assert!(!result.is_authorized());
    }

    #[test]
    fn test_trusted_issuer() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();
        let other_keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:tool").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        store.add(token.clone()).unwrap();

        // Validator that only trusts other_keypair
        let validator =
            CapabilityValidator::new(&store).trust_issuer(other_keypair.export_public_key());

        // Should fail - token not from trusted issuer
        assert!(validator.validate_token(&token).is_err());

        // Validator that trusts our keypair
        let validator2 = CapabilityValidator::new(&store).trust_issuer(keypair.export_public_key());

        assert!(validator2.validate_token(&token).is_ok());
    }

    #[test]
    fn test_multi_permission_check() {
        let store = CapabilityStore::in_memory();
        let keypair = test_keypair();

        let token = CapabilityToken::create(
            ResourcePattern::exact("mcp://test:read").unwrap(),
            vec![Permission::Invoke],
            TokenScope::Session,
            keypair.key_id(),
            AuditEntryId::new(),
            &keypair,
            None,
        );

        store.add(token).unwrap();

        let validator = CapabilityValidator::new(&store);

        let check = MultiPermissionCheck::new()
            .add("mcp://test:read", Permission::Invoke)
            .add("mcp://test:write", Permission::Invoke);

        assert!(!check.all_authorized(&validator));

        let needs = check.needs_approval(&validator);
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].0, "mcp://test:write");
    }
}
