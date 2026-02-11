//! Binary hash verification for MCP servers.
//!
//! Provides cryptographic verification of server binaries before execution
//! to prevent tampered or unauthorized code from running.

use astralis_crypto::ContentHash;
use std::path::Path;

use crate::error::{McpError, McpResult};

/// Result of binary verification.
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// Binary verified successfully.
    Verified {
        /// Path to the verified binary.
        path: String,
        /// Hash of the binary.
        hash: String,
    },
    /// No hash configured, verification skipped.
    Skipped {
        /// Reason for skipping.
        reason: String,
    },
    /// Verification failed.
    Failed {
        /// Expected hash.
        expected: String,
        /// Actual hash.
        actual: String,
    },
}

impl VerificationResult {
    /// Check if verification passed or was skipped.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Verified { .. } | Self::Skipped { .. })
    }

    /// Check if verification failed.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Verify a binary against an expected hash.
///
/// The hash should be in the format `sha256:<hex>` or `blake3:<hex>`.
/// If no algorithm prefix is provided, sha256 (blake3) is assumed.
///
/// # Errors
///
/// Returns an error if:
/// - The binary file cannot be read
/// - The hash format is invalid
pub fn verify_binary_hash(path: &Path, expected_hash: &str) -> McpResult<VerificationResult> {
    // Read the binary
    let binary_data = std::fs::read(path)?;

    // Compute the hash
    let actual_hash = ContentHash::hash(&binary_data);

    // Parse expected hash
    let (algorithm, expected_hex) = if let Some(rest) = expected_hash.strip_prefix("sha256:") {
        ("sha256", rest)
    } else if let Some(rest) = expected_hash.strip_prefix("blake3:") {
        ("blake3", rest)
    } else {
        // Default to sha256/blake3 (they're the same in our implementation)
        ("sha256", expected_hash)
    };

    // Compare
    let actual_hex = actual_hash.to_hex();

    if actual_hex == expected_hex {
        Ok(VerificationResult::Verified {
            path: path.display().to_string(),
            hash: format!("{algorithm}:{actual_hex}"),
        })
    } else {
        Ok(VerificationResult::Failed {
            expected: format!("{algorithm}:{expected_hex}"),
            actual: format!("{algorithm}:{actual_hex}"),
        })
    }
}

/// Find a binary in PATH and verify its hash.
///
/// # Errors
///
/// Returns an error if:
/// - The binary cannot be found in PATH
/// - The binary file cannot be read
pub fn verify_command_hash(command: &str, expected_hash: &str) -> McpResult<VerificationResult> {
    let binary_path = which::which(command)
        .map_err(|e| McpError::ConfigError(format!("Cannot find binary {command}: {e}")))?;

    verify_binary_hash(&binary_path, expected_hash)
}

/// Binary verifier for caching verification results.
#[derive(Debug, Default)]
pub struct BinaryVerifier {
    /// Cache of verified binaries (path -> hash).
    verified_cache: std::collections::HashMap<String, String>,
}

impl BinaryVerifier {
    /// Create a new verifier.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Verify a binary, using cache if available.
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails.
    pub fn verify(
        &mut self,
        path: &Path,
        expected_hash: Option<&str>,
    ) -> McpResult<VerificationResult> {
        let path_str = path.display().to_string();

        // Check if already verified
        if let Some(cached_hash) = self.verified_cache.get(&path_str) {
            if let Some(expected) = expected_hash {
                if cached_hash.ends_with(expected.split(':').next_back().unwrap_or(expected)) {
                    return Ok(VerificationResult::Verified {
                        path: path_str,
                        hash: cached_hash.clone(),
                    });
                }
            } else {
                return Ok(VerificationResult::Skipped {
                    reason: "already verified, no hash to check".to_string(),
                });
            }
        }

        // No expected hash means skip verification
        let Some(expected) = expected_hash else {
            return Ok(VerificationResult::Skipped {
                reason: "no hash configured".to_string(),
            });
        };

        // Verify the binary
        let result = verify_binary_hash(path, expected)?;

        // Cache successful verification
        if let VerificationResult::Verified { hash, .. } = &result {
            self.verified_cache.insert(path_str, hash.clone());
        }

        Ok(result)
    }

    /// Verify a command by name, looking it up in PATH.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be found or verification fails.
    pub fn verify_command(
        &mut self,
        command: &str,
        expected_hash: Option<&str>,
    ) -> McpResult<VerificationResult> {
        let binary_path = which::which(command)
            .map_err(|e| McpError::ConfigError(format!("Cannot find binary {command}: {e}")))?;

        self.verify(&binary_path, expected_hash)
    }

    /// Clear the verification cache.
    pub fn clear_cache(&mut self) {
        self.verified_cache.clear();
    }

    /// Get the number of cached verifications.
    #[must_use]
    pub fn cache_size(&self) -> usize {
        self.verified_cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_verification_result_is_ok() {
        assert!(
            VerificationResult::Verified {
                path: "test".into(),
                hash: "sha256:abc".into()
            }
            .is_ok()
        );

        assert!(
            VerificationResult::Skipped {
                reason: "test".into()
            }
            .is_ok()
        );

        assert!(
            !VerificationResult::Failed {
                expected: "sha256:abc".into(),
                actual: "sha256:def".into()
            }
            .is_ok()
        );
    }

    #[test]
    fn test_verify_binary_hash() {
        // Create a temp file with known content
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"test content").unwrap();
        file.flush().unwrap();

        // Compute expected hash
        let expected_hash = ContentHash::hash(b"test content");
        let expected_str = format!("sha256:{}", expected_hash.to_hex());

        let result = verify_binary_hash(file.path(), &expected_str).unwrap();

        assert!(matches!(result, VerificationResult::Verified { .. }));
    }

    #[test]
    fn test_verify_binary_hash_mismatch() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"test content").unwrap();
        file.flush().unwrap();

        let result = verify_binary_hash(file.path(), "sha256:0000000000000000").unwrap();

        assert!(matches!(result, VerificationResult::Failed { .. }));
    }

    #[test]
    fn test_verifier_cache() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"test content").unwrap();
        file.flush().unwrap();

        let expected_hash = ContentHash::hash(b"test content");
        let expected_str = format!("sha256:{}", expected_hash.to_hex());

        let mut verifier = BinaryVerifier::new();

        // First verification
        let result = verifier.verify(file.path(), Some(&expected_str)).unwrap();
        assert!(result.is_ok());
        assert_eq!(verifier.cache_size(), 1);

        // Second verification (from cache)
        let result = verifier.verify(file.path(), Some(&expected_str)).unwrap();
        assert!(result.is_ok());
        assert_eq!(verifier.cache_size(), 1);
    }

    #[test]
    fn test_verifier_no_hash() {
        let file = NamedTempFile::new().unwrap();
        let mut verifier = BinaryVerifier::new();

        let result = verifier.verify(file.path(), None).unwrap();

        assert!(matches!(result, VerificationResult::Skipped { .. }));
    }
}
