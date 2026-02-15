//! Signature verification with trusted key management.
//!
//! Provides a registry of trusted public keys for signature verification.

use std::collections::HashMap;

use crate::error::{CryptoError, CryptoResult};
use crate::keypair::PublicKey;
use crate::signature::Signature;

/// Key identifier (first 8 bytes of the public key).
pub type KeyId = [u8; 8];

/// A registry of trusted public keys for signature verification.
///
/// This struct maintains a set of trusted public keys and provides
/// methods for verifying signatures against those keys.
///
/// # Example
///
/// ```
/// use astrid_crypto::{KeyPair, SignatureVerifier};
///
/// let keypair = KeyPair::generate();
/// let mut verifier = SignatureVerifier::new();
///
/// // Add the public key as trusted
/// let key_id = verifier.add_trusted_key(keypair.export_public_key());
///
/// // Sign a message
/// let message = b"important data";
/// let signature = keypair.sign(message);
///
/// // Verify using the trusted key registry
/// assert!(verifier.verify(&key_id, message, &signature).is_ok());
/// ```
#[derive(Debug, Clone, Default)]
pub struct SignatureVerifier {
    /// Map of key IDs to trusted public keys.
    trusted_keys: HashMap<KeyId, PublicKey>,
}

impl SignatureVerifier {
    /// Create a new empty signature verifier.
    #[must_use]
    pub fn new() -> Self {
        Self {
            trusted_keys: HashMap::new(),
        }
    }

    /// Add a public key to the trusted key set.
    ///
    /// Returns the key ID (first 8 bytes of the public key) that can be
    /// used to reference this key later.
    pub fn add_trusted_key(&mut self, key: PublicKey) -> KeyId {
        let key_id = key.key_id();
        self.trusted_keys.insert(key_id, key);
        key_id
    }

    /// Remove a public key from the trusted key set.
    ///
    /// Returns `true` if the key was present and removed, `false` otherwise.
    pub fn remove_trusted_key(&mut self, key_id: &KeyId) -> bool {
        self.trusted_keys.remove(key_id).is_some()
    }

    /// Check if a key ID is in the trusted key set.
    #[must_use]
    pub fn is_trusted(&self, key_id: &KeyId) -> bool {
        self.trusted_keys.contains_key(key_id)
    }

    /// Get a trusted public key by its ID.
    #[must_use]
    pub fn get_key(&self, key_id: &KeyId) -> Option<&PublicKey> {
        self.trusted_keys.get(key_id)
    }

    /// Get the number of trusted keys.
    #[must_use]
    pub fn trusted_key_count(&self) -> usize {
        self.trusted_keys.len()
    }

    /// List all trusted key IDs.
    #[must_use]
    pub fn trusted_key_ids(&self) -> Vec<KeyId> {
        self.trusted_keys.keys().copied().collect()
    }

    /// Clear all trusted keys.
    pub fn clear(&mut self) {
        self.trusted_keys.clear();
    }

    /// Verify a signature using a trusted key.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The key ID is not in the trusted key set
    /// - The signature verification fails
    pub fn verify(
        &self,
        key_id: &KeyId,
        message: &[u8],
        signature: &Signature,
    ) -> CryptoResult<()> {
        let key = self
            .trusted_keys
            .get(key_id)
            .ok_or_else(|| CryptoError::InvalidPublicKey(format!("key {key_id:?} not trusted")))?;

        key.verify(message, signature)
    }

    /// Verify a signature using any trusted key that matches.
    ///
    /// This tries each trusted key until one successfully verifies the signature.
    /// Useful when you don't know which key was used to sign.
    ///
    /// # Errors
    ///
    /// Returns an error if no trusted key can verify the signature.
    pub fn verify_any(&self, message: &[u8], signature: &Signature) -> CryptoResult<KeyId> {
        for (key_id, key) in &self.trusted_keys {
            if key.verify(message, signature).is_ok() {
                return Ok(*key_id);
            }
        }

        Err(CryptoError::SignatureVerificationFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyPair;

    #[test]
    fn test_verifier_basic() {
        let keypair = KeyPair::generate();
        let mut verifier = SignatureVerifier::new();

        assert_eq!(verifier.trusted_key_count(), 0);

        let key_id = verifier.add_trusted_key(keypair.export_public_key());

        assert_eq!(verifier.trusted_key_count(), 1);
        assert!(verifier.is_trusted(&key_id));
        assert!(verifier.get_key(&key_id).is_some());
    }

    #[test]
    fn test_verifier_verify() {
        let keypair = KeyPair::generate();
        let mut verifier = SignatureVerifier::new();
        let key_id = verifier.add_trusted_key(keypair.export_public_key());

        let message = b"test message";
        let signature = keypair.sign(message);

        // Should succeed with correct message
        assert!(verifier.verify(&key_id, message, &signature).is_ok());

        // Should fail with wrong message
        assert!(
            verifier
                .verify(&key_id, b"wrong message", &signature)
                .is_err()
        );
    }

    #[test]
    fn test_verifier_untrusted_key() {
        let keypair = KeyPair::generate();
        let verifier = SignatureVerifier::new();

        let message = b"test message";
        let signature = keypair.sign(message);
        let key_id = keypair.key_id();

        // Should fail because key is not trusted
        assert!(verifier.verify(&key_id, message, &signature).is_err());
    }

    #[test]
    fn test_verifier_remove_key() {
        let keypair = KeyPair::generate();
        let mut verifier = SignatureVerifier::new();
        let key_id = verifier.add_trusted_key(keypair.export_public_key());

        assert!(verifier.is_trusted(&key_id));
        assert!(verifier.remove_trusted_key(&key_id));
        assert!(!verifier.is_trusted(&key_id));
        assert!(!verifier.remove_trusted_key(&key_id)); // Second removal returns false
    }

    #[test]
    fn test_verifier_verify_any() {
        let keypair1 = KeyPair::generate();
        let keypair2 = KeyPair::generate();
        let mut verifier = SignatureVerifier::new();

        let key_id1 = verifier.add_trusted_key(keypair1.export_public_key());
        let _key_id2 = verifier.add_trusted_key(keypair2.export_public_key());

        let message = b"test message";
        let signature = keypair1.sign(message);

        // Should find the correct key
        let found_key_id = verifier.verify_any(message, &signature).unwrap();
        assert_eq!(found_key_id, key_id1);
    }

    #[test]
    fn test_verifier_verify_any_no_match() {
        let keypair1 = KeyPair::generate();
        let keypair2 = KeyPair::generate();
        let mut verifier = SignatureVerifier::new();

        verifier.add_trusted_key(keypair1.export_public_key());

        let message = b"test message";
        let signature = keypair2.sign(message); // Signed by untrusted key

        // Should fail because signing key is not trusted
        assert!(verifier.verify_any(message, &signature).is_err());
    }

    #[test]
    fn test_verifier_clear() {
        let keypair = KeyPair::generate();
        let mut verifier = SignatureVerifier::new();

        verifier.add_trusted_key(keypair.export_public_key());
        assert_eq!(verifier.trusted_key_count(), 1);

        verifier.clear();
        assert_eq!(verifier.trusted_key_count(), 0);
    }

    #[test]
    fn test_verifier_list_keys() {
        let keypair1 = KeyPair::generate();
        let keypair2 = KeyPair::generate();
        let mut verifier = SignatureVerifier::new();

        let key_id1 = verifier.add_trusted_key(keypair1.export_public_key());
        let key_id2 = verifier.add_trusted_key(keypair2.export_public_key());

        let key_ids = verifier.trusted_key_ids();
        assert_eq!(key_ids.len(), 2);
        assert!(key_ids.contains(&key_id1));
        assert!(key_ids.contains(&key_id2));
    }
}
