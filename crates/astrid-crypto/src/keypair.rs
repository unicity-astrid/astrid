//! Ed25519 key pairs with secure memory handling.
//!
//! Provides key generation, signing, and verification for:
//! - Runtime identity (signs audit entries, capability tokens)
//! - User identity verification (optional user signing keys)

use std::io::Write;
use std::path::Path;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::{CryptoError, CryptoResult};
use crate::signature::Signature;

/// An Ed25519 key pair with secure memory handling.
///
/// The secret key is zeroized on drop to prevent leaking sensitive material.
#[derive(ZeroizeOnDrop)]
pub struct KeyPair {
    #[zeroize(skip)] // VerifyingKey doesn't implement Zeroize
    verifying_key: VerifyingKey,
    signing_key: SigningKey,
}

impl KeyPair {
    /// Generate a new random key pair.
    #[must_use]
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        Self {
            verifying_key,
            signing_key,
        }
    }

    /// Create from a secret key (32 bytes).
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidKeyLength`] if the slice is not exactly 32 bytes.
    pub fn from_secret_key(bytes: &[u8]) -> CryptoResult<Self> {
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyLength {
                expected: 32,
                actual: bytes.len(),
            });
        }

        let mut secret = [0u8; 32];
        secret.copy_from_slice(bytes);

        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        // Zeroize the temporary buffer
        secret.zeroize();

        Ok(Self {
            verifying_key,
            signing_key,
        })
    }

    /// Get the public key bytes (32 bytes).
    #[must_use]
    pub fn public_key_bytes(&self) -> &[u8; 32] {
        self.verifying_key.as_bytes()
    }

    /// Get a short key ID (first 8 bytes of public key).
    ///
    /// Useful for identifying keys in logs without exposing the full key.
    #[must_use]
    pub fn key_id(&self) -> [u8; 8] {
        let mut id = [0u8; 8];
        id.copy_from_slice(&self.public_key_bytes()[..8]);
        id
    }

    /// Get the key ID as a hex string.
    #[must_use]
    pub fn key_id_hex(&self) -> String {
        hex::encode(self.key_id())
    }

    /// Sign a message.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        let sig = self.signing_key.sign(message);
        Signature::from(sig)
    }

    /// Verify a signature (convenience method using our public key).
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::SignatureVerificationFailed`] if verification fails.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> CryptoResult<()> {
        signature.verify(message, self.public_key_bytes())
    }

    /// Export the public key for serialization.
    #[must_use]
    pub fn export_public_key(&self) -> PublicKey {
        PublicKey::from_bytes(*self.public_key_bytes())
    }

    /// Export the secret key bytes (careful - sensitive!).
    ///
    /// This should only be used for secure storage.
    #[must_use]
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Load an existing key from a file, or generate and save a new one.
    ///
    /// If the file exists, reads 32 bytes and reconstructs the key pair.
    /// If the file does not exist, generates a new key pair, writes it
    /// atomically with 0o600 permissions on Unix (no world-readable window).
    ///
    /// Creates parent directories if needed.
    ///
    /// # Security
    ///
    /// - On Unix, uses `O_CREAT | O_EXCL` (atomic create) with mode 0o600
    ///   to prevent TOCTOU races and world-readable windows.
    /// - Refuses to read key files that are symlinks (symlink attack protection).
    /// - File read buffers are wrapped in `Zeroizing<Vec<u8>>` so secret key
    ///   material is cleared from memory when no longer needed.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::IoError`] on I/O failures, symlink detection, or
    /// [`CryptoError::InvalidKeyLength`] if the file has wrong length.
    pub fn load_or_generate(path: impl AsRef<Path>) -> CryptoResult<Self> {
        let path = path.as_ref();

        // Create parent directories if needed.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CryptoError::IoError(e.to_string()))?;
        }

        // Attempt atomic creation first (Unix: O_CREAT | O_EXCL with mode 0o600).
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(mut file) => {
                    let kp = Self::generate();
                    file.write_all(&kp.secret_key_bytes())
                        .map_err(|e| CryptoError::IoError(e.to_string()))?;
                    return Ok(kp);
                },
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Fall through to the read path below.
                },
                Err(e) => return Err(CryptoError::IoError(e.to_string())),
            }
        }

        // Non-Unix: try existence check then write (best-effort).
        #[cfg(not(unix))]
        if !path.exists() {
            let kp = Self::generate();
            std::fs::write(path, kp.secret_key_bytes())
                .map_err(|e| CryptoError::IoError(e.to_string()))?;
            return Ok(kp);
        }

        // --- Read path ---

        // Refuse symlinks (prevents symlink attacks redirecting to another file).
        let meta =
            std::fs::symlink_metadata(path).map_err(|e| CryptoError::IoError(e.to_string()))?;
        if meta.file_type().is_symlink() {
            return Err(CryptoError::IoError(
                "refusing to read key file: path is a symlink".into(),
            ));
        }

        // Read with zeroizing wrapper so secret bytes are cleared on drop.
        let bytes =
            Zeroizing::new(std::fs::read(path).map_err(|e| CryptoError::IoError(e.to_string()))?);
        Self::from_secret_key(&bytes)
    }

    /// Load or generate a key, returning two independent `KeyPair` instances
    /// from a single disk read.
    ///
    /// This avoids the pattern of calling [`load_or_generate`](Self::load_or_generate)
    /// twice (which reads the file twice and creates two un-zeroized intermediate
    /// buffers). Useful when separate `KeyPair` values are needed for different
    /// components (e.g., audit log + runtime) since `KeyPair` is not `Clone`.
    ///
    /// # Errors
    ///
    /// Same as [`load_or_generate`](Self::load_or_generate).
    pub fn load_or_generate_pair(path: impl AsRef<Path>) -> CryptoResult<(Self, Self)> {
        let first = Self::load_or_generate(path.as_ref())?;
        // The file now definitely exists; read it once more with zeroization.
        let meta = std::fs::symlink_metadata(path.as_ref())
            .map_err(|e| CryptoError::IoError(e.to_string()))?;
        if meta.file_type().is_symlink() {
            return Err(CryptoError::IoError(
                "refusing to read key file: path is a symlink".into(),
            ));
        }
        let bytes = Zeroizing::new(
            std::fs::read(path.as_ref()).map_err(|e| CryptoError::IoError(e.to_string()))?,
        );
        let second = Self::from_secret_key(&bytes)?;
        Ok((first, second))
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPair")
            .field("key_id", &self.key_id_hex())
            .finish_non_exhaustive()
    }
}

/// A public key (safe to share, serialize, etc.).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// Create from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Try to create from a slice.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidKeyLength`] if the slice is not exactly 32 bytes.
    pub fn try_from_slice(slice: &[u8]) -> CryptoResult<Self> {
        if slice.len() != 32 {
            return Err(CryptoError::InvalidKeyLength {
                expected: 32,
                actual: slice.len(),
            });
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    /// Get the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Get a short key ID (first 8 bytes).
    #[must_use]
    pub fn key_id(&self) -> [u8; 8] {
        let mut id = [0u8; 8];
        id.copy_from_slice(&self.0[..8]);
        id
    }

    /// Get the key ID as a hex string.
    #[must_use]
    pub fn key_id_hex(&self) -> String {
        hex::encode(self.key_id())
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
    pub fn from_hex(s: &str) -> CryptoResult<Self> {
        let bytes = hex::decode(s).map_err(|_| CryptoError::InvalidHexEncoding)?;
        Self::try_from_slice(&bytes)
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
    pub fn from_base64(s: &str) -> CryptoResult<Self> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|_| CryptoError::InvalidBase64Encoding)?;
        Self::try_from_slice(&bytes)
    }

    /// Verify a signature against this public key.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::SignatureVerificationFailed`] if verification fails.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> CryptoResult<()> {
        signature.verify(message, &self.0)
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({})", self.key_id_hex())
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_base64(&s).map_err(serde::de::Error::custom)
    }
}

impl From<[u8; 32]> for PublicKey {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<PublicKey> for [u8; 32] {
    fn from(pk: PublicKey) -> Self {
        pk.0
    }
}

impl AsRef<[u8]> for PublicKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypair_generation() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();

        // Different keypairs have different public keys
        assert_ne!(kp1.public_key_bytes(), kp2.public_key_bytes());
    }

    #[test]
    fn test_keypair_from_secret() {
        let original = KeyPair::generate();
        let secret = original.secret_key_bytes();

        let restored = KeyPair::from_secret_key(&secret).unwrap();

        assert_eq!(original.public_key_bytes(), restored.public_key_bytes());
    }

    #[test]
    fn test_sign_verify() {
        let keypair = KeyPair::generate();
        let message = b"hello world";

        let signature = keypair.sign(message);
        assert!(keypair.verify(message, &signature).is_ok());

        // Wrong message fails
        assert!(keypair.verify(b"wrong", &signature).is_err());
    }

    #[test]
    fn test_key_id() {
        let keypair = KeyPair::generate();
        let key_id = keypair.key_id();

        // Key ID is first 8 bytes of public key
        assert_eq!(&key_id[..], &keypair.public_key_bytes()[..8]);

        // Hex encoding works
        let hex_id = keypair.key_id_hex();
        assert_eq!(hex_id.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_public_key_encoding() {
        let keypair = KeyPair::generate();
        let pk = keypair.export_public_key();

        // Hex roundtrip
        let hex = pk.to_hex();
        let decoded = PublicKey::from_hex(&hex).unwrap();
        assert_eq!(pk, decoded);

        // Base64 roundtrip
        let b64 = pk.to_base64();
        let decoded = PublicKey::from_base64(&b64).unwrap();
        assert_eq!(pk, decoded);
    }

    #[test]
    fn test_public_key_verify() {
        let keypair = KeyPair::generate();
        let pk = keypair.export_public_key();
        let message = b"test";

        let sig = keypair.sign(message);
        assert!(pk.verify(message, &sig).is_ok());
    }

    #[test]
    fn test_invalid_key_length() {
        let result = KeyPair::from_secret_key(&[0u8; 31]);
        assert!(matches!(result, Err(CryptoError::InvalidKeyLength { .. })));
    }

    #[test]
    fn test_load_or_generate_creates_new() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys").join("test.key");

        let kp1 = KeyPair::load_or_generate(&path).unwrap();
        assert!(path.exists());

        // Reload returns same public key
        let kp2 = KeyPair::load_or_generate(&path).unwrap();
        assert_eq!(kp1.public_key_bytes(), kp2.public_key_bytes());
    }

    #[test]
    fn test_load_or_generate_rejects_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.key");

        // Write wrong-length file
        std::fs::write(&path, &[0u8; 16]).unwrap();

        let result = KeyPair::load_or_generate(&path);
        assert!(matches!(result, Err(CryptoError::InvalidKeyLength { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn test_load_or_generate_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secure.key");

        KeyPair::load_or_generate(&path).unwrap();

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn test_load_or_generate_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real_path = dir.path().join("real.key");
        let link_path = dir.path().join("link.key");

        // Create a real key file, then a symlink to it
        KeyPair::load_or_generate(&real_path).unwrap();
        std::os::unix::fs::symlink(&real_path, &link_path).unwrap();

        // Attempting to load via symlink should fail
        let result = KeyPair::load_or_generate(&link_path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("symlink"),
            "expected symlink error, got: {err}"
        );
    }

    #[test]
    fn test_load_or_generate_pair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys").join("pair.key");

        let (kp1, kp2) = KeyPair::load_or_generate_pair(&path).unwrap();
        assert_eq!(kp1.public_key_bytes(), kp2.public_key_bytes());

        // Both should produce valid signatures
        let msg = b"test message";
        let sig1 = kp1.sign(msg);
        let sig2 = kp2.sign(msg);
        assert!(kp1.verify(msg, &sig1).is_ok());
        assert!(kp2.verify(msg, &sig2).is_ok());
    }
}
