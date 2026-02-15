//! SRI (Subresource Integrity) hash verification for npm tarballs.
//!
//! Verifies `sha512-<base64>` and `sha256-<base64>` integrity strings from
//! npm registry metadata. SHA-1 is deliberately rejected (cryptographically broken).

use sha2::{Digest, Sha256, Sha512};
use subtle::ConstantTimeEq;

use crate::error::{PluginError, PluginResult};

/// Verify data against an SRI integrity string.
///
/// Supports `sha512-` and `sha256-` prefixed SRI hashes. SHA-1 is rejected.
/// Multiple hashes separated by spaces are supported — the strongest is used.
///
/// # Errors
///
/// Returns `PluginError::IntegrityError` on hash mismatch,
/// `PluginError::RegistryError` on malformed SRI strings.
pub fn verify_sri_integrity(data: &[u8], sri: &str, package_name: &str) -> PluginResult<()> {
    // SRI can contain multiple space-separated hashes — pick the strongest.
    let hash = pick_strongest_hash(sri)?;
    let (algorithm, expected_b64) = hash;

    let expected_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, expected_b64).map_err(
            |e| PluginError::RegistryError {
                message: format!("invalid base64 in SRI hash: {e}"),
            },
        )?;

    let actual_bytes = compute_hash(algorithm, data);

    // Constant-time comparison to avoid timing side-channels.
    // Uses the `subtle` crate which provides compiler-barrier-protected
    // constant-time operations that LLVM cannot optimize into early-exit.
    if !bool::from(actual_bytes.ct_eq(&expected_bytes)) {
        return Err(PluginError::IntegrityError {
            package: package_name.to_string(),
            expected: sri.to_string(),
        });
    }

    Ok(())
}

/// Supported SRI hash algorithms, ordered by strength.
///
/// SHA-1 is deliberately excluded — it is cryptographically broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SriAlgorithm {
    Sha256,
    Sha512,
}

/// Parse an SRI string and pick the strongest hash.
///
/// SHA-1 tokens are silently skipped (SHA-1 is cryptographically broken).
fn pick_strongest_hash(sri: &str) -> PluginResult<(SriAlgorithm, &str)> {
    let mut best: Option<(SriAlgorithm, &str)> = None;

    for part in sri.split_whitespace() {
        if let Some((algo, hash)) = parse_single_sri(part)?
            && best.as_ref().is_none_or(|(best_algo, _)| algo > *best_algo)
        {
            best = Some((algo, hash));
        }
    }

    best.ok_or_else(|| PluginError::RegistryError {
        message: format!("no valid hash found in SRI string: {sri}"),
    })
}

/// Parse a single `algorithm-hash` SRI token.
///
/// Returns `Ok(None)` for SHA-1 (deliberately rejected — SHA-1 is broken).
/// Returns `Err` for completely unknown algorithms.
fn parse_single_sri(token: &str) -> PluginResult<Option<(SriAlgorithm, &str)>> {
    // Handle optional `?opt` parameters per SRI spec.
    let token = token.split('?').next().unwrap_or(token);

    if let Some(hash) = token.strip_prefix("sha512-") {
        Ok(Some((SriAlgorithm::Sha512, hash)))
    } else if let Some(hash) = token.strip_prefix("sha256-") {
        Ok(Some((SriAlgorithm::Sha256, hash)))
    } else if token.strip_prefix("sha1-").is_some() {
        // SHA-1 is cryptographically broken — skip it entirely.
        // If the SRI string only contains sha1, pick_strongest_hash
        // will return "no valid hash found", which is the safe default.
        Ok(None)
    } else {
        Err(PluginError::RegistryError {
            message: format!("unsupported SRI algorithm in: {token}"),
        })
    }
}

/// Compute a hash of data using the specified algorithm.
fn compute_hash(algorithm: SriAlgorithm, data: &[u8]) -> Vec<u8> {
    match algorithm {
        SriAlgorithm::Sha512 => Sha512::digest(data).to_vec(),
        SriAlgorithm::Sha256 => Sha256::digest(data).to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha512_sri(data: &[u8]) -> String {
        use base64::Engine;
        let digest = Sha512::digest(data);
        let b64 = base64::engine::general_purpose::STANDARD.encode(digest);
        format!("sha512-{b64}")
    }

    fn sha256_sri(data: &[u8]) -> String {
        use base64::Engine;
        let digest = Sha256::digest(data);
        let b64 = base64::engine::general_purpose::STANDARD.encode(digest);
        format!("sha256-{b64}")
    }

    #[test]
    fn verify_sha512_valid() {
        let data = b"hello world";
        let sri = sha512_sri(data);
        verify_sri_integrity(data, &sri, "test-pkg").unwrap();
    }

    #[test]
    fn verify_sha256_valid() {
        let data = b"hello world";
        let sri = sha256_sri(data);
        verify_sri_integrity(data, &sri, "test-pkg").unwrap();
    }

    #[test]
    fn verify_sha512_wrong_data() {
        let sri = sha512_sri(b"hello world");
        let err = verify_sri_integrity(b"wrong data", &sri, "test-pkg").unwrap_err();
        assert!(err.to_string().contains("integrity mismatch"));
    }

    #[test]
    fn verify_picks_strongest() {
        // Provide both sha256 and sha512 — sha512 should win.
        let data = b"test data";
        let sri = format!("{} {}", sha256_sri(data), sha512_sri(data));
        verify_sri_integrity(data, &sri, "test-pkg").unwrap();
    }

    #[test]
    fn verify_invalid_base64() {
        let err = verify_sri_integrity(b"data", "sha512-!!!invalid!!!", "test-pkg").unwrap_err();
        assert!(err.to_string().contains("invalid base64"));
    }

    #[test]
    fn verify_unknown_algorithm() {
        let err = verify_sri_integrity(b"data", "md5-abc123", "test-pkg").unwrap_err();
        assert!(err.to_string().contains("unsupported SRI algorithm"));
    }

    #[test]
    fn verify_empty_sri() {
        let err = verify_sri_integrity(b"data", "", "test-pkg").unwrap_err();
        assert!(err.to_string().contains("no valid hash"));
    }

    #[test]
    fn sha1_deliberately_rejected() {
        // SHA-1 is broken — skipped at parse time, so sha1-only SRI yields
        // "no valid hash found" rather than a silent mismatch.
        let err = verify_sri_integrity(b"data", "sha1-abc123", "test-pkg").unwrap_err();
        assert!(
            err.to_string().contains("no valid hash"),
            "expected 'no valid hash' error, got: {err}"
        );
    }

    #[test]
    fn sha1_skipped_when_stronger_available() {
        // If both sha1 and sha512 are present, sha512 is used and sha1 is ignored.
        let data = b"test data";
        let sri = format!("sha1-fakehash {}", sha512_sri(data));
        verify_sri_integrity(data, &sri, "test-pkg").unwrap();
    }
}
