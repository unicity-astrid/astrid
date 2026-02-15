//! Content hashing using BLAKE3.
//!
//! Provides fast, cryptographic content hashing for audit chains,
//! content verification, and integrity checking.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A BLAKE3 content hash (32 bytes).
///
/// Used for:
/// - Audit chain linking (each entry hashes the previous)
/// - Content deduplication and verification
/// - Binary hash verification before execution
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hash arbitrary data.
    #[must_use]
    pub fn hash(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    /// Hash multiple data chunks (concatenated).
    #[must_use]
    pub fn hash_multi(parts: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        for part in parts {
            hasher.update(part);
        }
        Self(*hasher.finalize().as_bytes())
    }

    /// Create a zero hash (used for genesis entries).
    #[must_use]
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }

    /// Check if this is the zero hash.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Get the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Create from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Try to create from a slice.
    ///
    /// Returns `None` if the slice is not exactly 32 bytes.
    #[must_use]
    pub fn try_from_slice(slice: &[u8]) -> Option<Self> {
        if slice.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(slice);
        Some(Self(bytes))
    }

    /// Encode as hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Decode from hex string.
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not valid hex or not 32 bytes.
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        Self::try_from_slice(&bytes).ok_or(hex::FromHexError::InvalidStringLength)
    }

    /// Encode as base64 string.
    #[must_use]
    pub fn to_base64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(self.0)
    }

    /// Decode from base64 string.
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not valid base64 or not 32 bytes.
    pub fn from_base64(s: &str) -> Result<Self, base64::DecodeError> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(s)?;
        Self::try_from_slice(&bytes).ok_or(base64::DecodeError::InvalidLength(bytes.len()))
    }

    /// Create a hash with a prefix (for domain separation).
    ///
    /// # Example
    ///
    /// ```
    /// use astrid_crypto::ContentHash;
    ///
    /// let hash = ContentHash::hash_with_domain("audit-entry", b"data");
    /// ```
    #[must_use]
    pub fn hash_with_domain(domain: &str, data: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key(domain);
        hasher.update(data);
        Self(*hasher.finalize().as_bytes())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", &self.to_hex()[..16])
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for ContentHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

impl Default for ContentHash {
    fn default() -> Self {
        Self::zero()
    }
}

impl AsRef<[u8]> for ContentHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; 32]> for ContentHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<ContentHash> for [u8; 32] {
    fn from(hash: ContentHash) -> Self {
        hash.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_basic() {
        let data = b"hello world";
        let hash = ContentHash::hash(data);

        // Same data produces same hash
        assert_eq!(hash, ContentHash::hash(data));

        // Different data produces different hash
        assert_ne!(hash, ContentHash::hash(b"different"));
    }

    #[test]
    fn test_hash_multi() {
        let parts: &[&[u8]] = &[b"hello", b" ", b"world"];
        let hash_multi = ContentHash::hash_multi(parts);
        let hash_single = ContentHash::hash(b"hello world");

        assert_eq!(hash_multi, hash_single);
    }

    #[test]
    fn test_zero_hash() {
        let zero = ContentHash::zero();
        assert!(zero.is_zero());
        assert!(!ContentHash::hash(b"data").is_zero());
    }

    #[test]
    fn test_hex_encoding() {
        let hash = ContentHash::hash(b"test");
        let hex = hash.to_hex();
        let decoded = ContentHash::from_hex(&hex).unwrap();
        assert_eq!(hash, decoded);
    }

    #[test]
    fn test_base64_encoding() {
        let hash = ContentHash::hash(b"test");
        let b64 = hash.to_base64();
        let decoded = ContentHash::from_base64(&b64).unwrap();
        assert_eq!(hash, decoded);
    }

    #[test]
    fn test_domain_separation() {
        let data = b"same data";
        let hash1 = ContentHash::hash_with_domain("domain1", data);
        let hash2 = ContentHash::hash_with_domain("domain2", data);

        // Same data, different domains = different hashes
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_serde() {
        let hash = ContentHash::hash(b"test");
        let json = serde_json::to_string(&hash).unwrap();
        let decoded: ContentHash = serde_json::from_str(&json).unwrap();
        assert_eq!(hash, decoded);
    }
}
