//! Audit log - main interface for audit logging.
//!
//! Provides a high-level API for recording and verifying audit entries.

use astrid_capabilities::AuditEntryId;
use astrid_core::SessionId;
use astrid_crypto::{ContentHash, KeyPair};
use std::path::Path;
use std::sync::RwLock;
use tracing::{debug, error, warn};

use crate::entry::{AuditAction, AuditEntry, AuditOutcome, AuthorizationProof};
use crate::error::{AuditError, AuditResult};
use crate::storage::{AuditStorage, SurrealKvAuditStorage};

/// Key for the per-chain head cache: (session, optional principal).
///
/// System entries (no principal) use `(session_id, None)`.
/// Principal entries use `(session_id, Some(principal))`.
type ChainKey = (SessionId, Option<astrid_core::PrincipalId>);

/// Audit log for recording and verifying security events.
pub struct AuditLog {
    /// Storage backend.
    storage: Box<dyn AuditStorage>,
    /// Runtime signing key.
    runtime_key: KeyPair,
    /// Current chain heads per (session, principal) pair.
    ///
    /// Each principal maintains its own independent chain within a session.
    /// System entries (no principal) use `(session_id, None)`.
    chain_heads: RwLock<std::collections::HashMap<ChainKey, ContentHash>>,
}

impl AuditLog {
    /// Create a new audit log with `SurrealKV` persistence.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails to open at the given path.
    pub fn open(path: impl AsRef<Path>, runtime_key: KeyPair) -> AuditResult<Self> {
        let storage = SurrealKvAuditStorage::open(path)?;
        Ok(Self {
            storage: Box::new(storage),
            runtime_key,
            chain_heads: RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Create an in-memory audit log (for testing).
    #[must_use]
    pub fn in_memory(runtime_key: KeyPair) -> Self {
        let storage = SurrealKvAuditStorage::in_memory();
        Self {
            storage: Box::new(storage),
            runtime_key,
            chain_heads: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Append a new audit entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry cannot be stored or the chain head cannot be updated.
    pub fn append(
        &self,
        session_id: SessionId,
        action: AuditAction,
        authorization: AuthorizationProof,
        outcome: AuditOutcome,
    ) -> AuditResult<AuditEntryId> {
        self.append_inner(session_id, None, action, authorization, outcome)
    }

    /// Append a new audit entry tagged with the acting principal.
    ///
    /// Use this when the action was performed on behalf of a specific
    /// user (e.g., cross-principal KV write, tool execution). The
    /// principal is included in the cryptographic signing data.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry cannot be stored or the chain head cannot be updated.
    pub fn append_with_principal(
        &self,
        session_id: SessionId,
        principal: astrid_core::PrincipalId,
        action: AuditAction,
        authorization: AuthorizationProof,
        outcome: AuditOutcome,
    ) -> AuditResult<AuditEntryId> {
        self.append_inner(session_id, Some(principal), action, authorization, outcome)
    }

    /// Shared implementation for `append` and `append_with_principal`.
    fn append_inner(
        &self,
        session_id: SessionId,
        principal: Option<astrid_core::PrincipalId>,
        action: AuditAction,
        authorization: AuthorizationProof,
        outcome: AuditOutcome,
    ) -> AuditResult<AuditEntryId> {
        // Get the previous hash for this entry's chain (system or principal).
        let chain_key: ChainKey = (session_id.clone(), principal.clone());
        let previous_hash = self.get_previous_hash(&chain_key)?;

        // Create and sign the entry. session_id is moved into create,
        // chain_key retains the clone for the cache update below.
        let entry = if let Some(p) = principal {
            AuditEntry::create_with_principal(
                session_id,
                p,
                action,
                authorization,
                outcome,
                previous_hash,
                &self.runtime_key,
            )
        } else {
            AuditEntry::create(
                session_id,
                action,
                authorization,
                outcome,
                previous_hash,
                &self.runtime_key,
            )
        };

        let entry_id = entry.id.clone();
        let entry_hash = entry.content_hash();

        debug!(
            entry_id = %entry_id,
            action = %entry.action.description(),
            "Appending audit entry"
        );

        // Store the entry
        self.storage.store(&entry)?;

        // Update cached chain head for this entry's chain.
        {
            let mut heads = self
                .chain_heads
                .write()
                .map_err(|e| AuditError::StorageError(e.to_string()))?;
            heads.insert(chain_key, entry_hash);
        }

        Ok(entry_id)
    }

    /// Get the previous hash for a chain (session + optional principal).
    fn get_previous_hash(&self, chain_key: &ChainKey) -> AuditResult<ContentHash> {
        // Check cache first
        {
            let heads = self
                .chain_heads
                .read()
                .map_err(|e| AuditError::StorageError(e.to_string()))?;
            if let Some(hash) = heads.get(chain_key) {
                return Ok(*hash);
            }
        }

        // Check storage
        if let Some(head_id) = self
            .storage
            .get_chain_head(&chain_key.0, chain_key.1.as_ref())?
            && let Some(entry) = self.storage.get(&head_id)?
        {
            return Ok(entry.content_hash());
        }

        // Genesis - no previous entry for this chain
        Ok(ContentHash::zero())
    }

    /// Get an entry by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails to retrieve the entry.
    pub fn get(&self, id: &AuditEntryId) -> AuditResult<Option<AuditEntry>> {
        self.storage.get(id)
    }

    /// Get all entries for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails to retrieve entries.
    pub fn get_session_entries(&self, session_id: &SessionId) -> AuditResult<Vec<AuditEntry>> {
        self.storage.get_session_entries(session_id)
    }

    /// Verify the integrity of all audit chains in a session.
    ///
    /// Each principal (and the system chain) is verified independently.
    /// A session with entries from principals "alice" and "bob" plus system
    /// entries will verify three independent chains.
    ///
    /// # Errors
    ///
    /// Returns an error if entries cannot be retrieved from storage.
    pub fn verify_chain(&self, session_id: &SessionId) -> AuditResult<ChainVerificationResult> {
        let entries = self.storage.get_session_entries(session_id)?;

        if entries.is_empty() {
            return Ok(ChainVerificationResult {
                valid: true,
                entries_verified: 0,
                issues: Vec::new(),
            });
        }

        // Group entries by principal (None = system chain).
        let mut chains: std::collections::HashMap<
            Option<astrid_core::PrincipalId>,
            Vec<&AuditEntry>,
        > = std::collections::HashMap::new();
        for entry in &entries {
            chains
                .entry(entry.principal.clone())
                .or_default()
                .push(entry);
        }

        let mut issues = Vec::new();
        let mut entries_verified: usize = 0;

        // Verify each chain independently.
        for chain_entries in chains.values_mut() {
            // Sort by timestamp within each chain.
            chain_entries.sort_by(|a, b| a.timestamp.0.cmp(&b.timestamp.0));

            // Verify genesis (first entry has zero previous hash).
            if !chain_entries[0].previous_hash.is_zero() {
                issues.push(ChainIssue::InvalidGenesis {
                    entry_id: chain_entries[0].id.clone(),
                });
            }

            // Verify signatures.
            for entry in chain_entries.iter() {
                if let Err(e) = entry.verify_signature() {
                    error!(entry_id = %entry.id, error = %e, "Invalid signature");
                    issues.push(ChainIssue::InvalidSignature {
                        entry_id: entry.id.clone(),
                    });
                }
                entries_verified = entries_verified.saturating_add(1);
            }

            // Verify chain linking within this principal's chain.
            for i in 1..chain_entries.len() {
                #[expect(clippy::arithmetic_side_effects)]
                let prev = chain_entries[i - 1];
                let curr = chain_entries[i];

                if !curr.follows(prev) {
                    warn!(
                        current = %curr.id,
                        previous = %prev.id,
                        "Chain link broken"
                    );
                    issues.push(ChainIssue::BrokenLink {
                        entry_id: curr.id.clone(),
                        expected_previous: prev.content_hash(),
                        actual_previous: curr.previous_hash,
                    });
                }
            }
        }

        Ok(ChainVerificationResult {
            valid: issues.is_empty(),
            entries_verified,
            issues,
        })
    }

    /// Verify the integrity of a single principal's chain within a session.
    ///
    /// Pass `None` to verify the system chain (entries without a principal).
    ///
    /// # Errors
    ///
    /// Returns an error if entries cannot be retrieved from storage.
    pub fn verify_principal_chain(
        &self,
        session_id: &SessionId,
        principal: Option<&astrid_core::PrincipalId>,
    ) -> AuditResult<ChainVerificationResult> {
        let entries = self.get_principal_entries(session_id, principal)?;

        if entries.is_empty() {
            return Ok(ChainVerificationResult {
                valid: true,
                entries_verified: 0,
                issues: Vec::new(),
            });
        }

        let mut issues = Vec::new();
        let mut entries_verified: usize = 0;

        let mut sorted = entries;
        sorted.sort_by(|a, b| a.timestamp.0.cmp(&b.timestamp.0));

        if !sorted[0].previous_hash.is_zero() {
            issues.push(ChainIssue::InvalidGenesis {
                entry_id: sorted[0].id.clone(),
            });
        }

        for entry in &sorted {
            if let Err(e) = entry.verify_signature() {
                error!(entry_id = %entry.id, error = %e, "Invalid signature");
                issues.push(ChainIssue::InvalidSignature {
                    entry_id: entry.id.clone(),
                });
            }
            entries_verified = entries_verified.saturating_add(1);
        }

        for i in 1..sorted.len() {
            #[expect(clippy::arithmetic_side_effects)]
            let prev = &sorted[i - 1];
            let curr = &sorted[i];
            if !curr.follows(prev) {
                warn!(current = %curr.id, previous = %prev.id, "Chain link broken");
                issues.push(ChainIssue::BrokenLink {
                    entry_id: curr.id.clone(),
                    expected_previous: prev.content_hash(),
                    actual_previous: curr.previous_hash,
                });
            }
        }

        Ok(ChainVerificationResult {
            valid: issues.is_empty(),
            entries_verified,
            issues,
        })
    }

    /// Get entries for a specific principal within a session.
    ///
    /// Pass `None` to get system entries (no principal).
    ///
    /// # Errors
    ///
    /// Returns an error if entries cannot be retrieved from storage.
    pub fn get_principal_entries(
        &self,
        session_id: &SessionId,
        principal: Option<&astrid_core::PrincipalId>,
    ) -> AuditResult<Vec<AuditEntry>> {
        let all = self.storage.get_session_entries(session_id)?;
        Ok(all
            .into_iter()
            .filter(|e| e.principal.as_ref() == principal)
            .collect())
    }

    /// Verify the entire audit log (all sessions).
    ///
    /// # Errors
    ///
    /// Returns an error if sessions cannot be listed or verified.
    pub fn verify_all(&self) -> AuditResult<Vec<(SessionId, ChainVerificationResult)>> {
        let sessions = self.storage.list_sessions()?;
        let mut results = Vec::new();

        for session_id in sessions {
            let result = self.verify_chain(&session_id)?;
            results.push((session_id, result));
        }

        Ok(results)
    }

    /// Count total entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails.
    pub fn count(&self) -> AuditResult<usize> {
        self.storage.count()
    }

    /// Count entries for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails.
    pub fn count_session(&self, session_id: &SessionId) -> AuditResult<usize> {
        self.storage.count_session(session_id)
    }

    /// List all sessions.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails.
    pub fn list_sessions(&self) -> AuditResult<Vec<SessionId>> {
        self.storage.list_sessions()
    }

    /// Flush pending writes.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails to flush.
    pub fn flush(&self) -> AuditResult<()> {
        self.storage.flush()
    }

    /// Get the runtime public key.
    #[must_use]
    pub fn runtime_public_key(&self) -> astrid_crypto::PublicKey {
        self.runtime_key.export_public_key()
    }
}

impl std::fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditLog")
            .field("runtime_key_id", &self.runtime_key.key_id_hex())
            .finish_non_exhaustive()
    }
}

/// Result of chain verification.
#[derive(Debug, Clone)]
pub struct ChainVerificationResult {
    /// Whether the chain is valid.
    pub valid: bool,
    /// Number of entries verified.
    pub entries_verified: usize,
    /// Issues found (empty if valid).
    pub issues: Vec<ChainIssue>,
}

/// An issue found during chain verification.
#[derive(Debug, Clone)]
pub enum ChainIssue {
    /// First entry doesn't have zero previous hash.
    InvalidGenesis {
        /// The entry with invalid genesis.
        entry_id: AuditEntryId,
    },
    /// Entry has invalid signature.
    InvalidSignature {
        /// The entry with invalid signature.
        entry_id: AuditEntryId,
    },
    /// Chain link is broken.
    BrokenLink {
        /// The entry with broken link.
        entry_id: AuditEntryId,
        /// Expected previous hash.
        expected_previous: ContentHash,
        /// Actual previous hash in entry.
        actual_previous: ContentHash,
    },
}

impl std::fmt::Display for ChainIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGenesis { entry_id } => {
                write!(f, "Invalid genesis at {entry_id}")
            },
            Self::InvalidSignature { entry_id } => {
                write!(f, "Invalid signature at {entry_id}")
            },
            Self::BrokenLink { entry_id, .. } => {
                write!(f, "Broken chain link at {entry_id}")
            },
        }
    }
}

/// Builder for audit entries with fluent API.
#[cfg(test)]
pub(crate) struct AuditBuilder<'a> {
    log: &'a AuditLog,
    session_id: SessionId,
    action: Option<AuditAction>,
    authorization: Option<AuthorizationProof>,
}

#[cfg(test)]
impl<'a> AuditBuilder<'a> {
    /// Create a new audit builder.
    pub(crate) fn new(log: &'a AuditLog, session_id: SessionId) -> Self {
        Self {
            log,
            session_id,
            action: None,
            authorization: None,
        }
    }

    /// Set the action.
    #[must_use]
    pub(crate) fn action(mut self, action: AuditAction) -> Self {
        self.action = Some(action);
        self
    }

    /// Set the authorization.
    #[must_use]
    pub(crate) fn authorization(mut self, auth: AuthorizationProof) -> Self {
        self.authorization = Some(auth);
        self
    }

    /// Record success.
    ///
    /// # Panics
    ///
    /// Panics if `action` was not set on the builder.
    ///
    /// # Errors
    ///
    /// Returns an error if the audit entry cannot be appended.
    pub(crate) fn success(self) -> AuditResult<AuditEntryId> {
        self.log.append(
            self.session_id,
            self.action.expect("action required"),
            self.authorization
                .unwrap_or(AuthorizationProof::NotRequired {
                    reason: "unspecified".to_string(),
                }),
            AuditOutcome::success(),
        )
    }

    /// Record success with details.
    ///
    /// # Panics
    ///
    /// Panics if `action` was not set on the builder.
    ///
    /// # Errors
    ///
    /// Returns an error if the audit entry cannot be appended.
    pub(crate) fn success_with(self, details: impl Into<String>) -> AuditResult<AuditEntryId> {
        self.log.append(
            self.session_id,
            self.action.expect("action required"),
            self.authorization
                .unwrap_or(AuthorizationProof::NotRequired {
                    reason: "unspecified".to_string(),
                }),
            AuditOutcome::success_with(details),
        )
    }

    /// Record failure.
    ///
    /// # Panics
    ///
    /// Panics if `action` was not set on the builder.
    ///
    /// # Errors
    ///
    /// Returns an error if the audit entry cannot be appended.
    pub(crate) fn failure(self, error: impl Into<String>) -> AuditResult<AuditEntryId> {
        self.log.append(
            self.session_id,
            self.action.expect("action required"),
            self.authorization
                .unwrap_or(AuthorizationProof::NotRequired {
                    reason: "unspecified".to_string(),
                }),
            AuditOutcome::failure(error),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Append `count` test entries to the log, returning their IDs.
    fn append_test_entries(
        log: &AuditLog,
        session_id: &SessionId,
        count: u32,
    ) -> Vec<AuditEntryId> {
        (0..count)
            .map(|i| {
                log.append(
                    session_id.clone(),
                    AuditAction::McpToolCall {
                        server: "test".to_string(),
                        tool: format!("tool_{i}"),
                        args_hash: ContentHash::zero(),
                    },
                    AuthorizationProof::NotRequired {
                        reason: "test".to_string(),
                    },
                    AuditOutcome::success(),
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn test_append_and_retrieve() {
        let keypair = KeyPair::generate();
        let user_id = keypair.key_id();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();

        let entry_id = log
            .append(
                session_id.clone(),
                AuditAction::SessionStarted {
                    user_id,
                    platform: "cli".to_string(),
                },
                AuthorizationProof::System {
                    reason: "test".to_string(),
                },
                AuditOutcome::success(),
            )
            .unwrap();

        let entry = log.get(&entry_id).unwrap().unwrap();
        assert_eq!(entry.id, entry_id);
    }

    #[test]
    fn test_chain_verification() {
        let keypair = KeyPair::generate();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();

        append_test_entries(&log, &session_id, 5);

        let result = log.verify_chain(&session_id).unwrap();
        assert!(result.valid);
        assert_eq!(result.entries_verified, 5);
    }

    #[test]
    fn test_audit_builder() {
        let keypair = KeyPair::generate();
        let user_id = keypair.key_id();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();

        let entry_id = AuditBuilder::new(&log, session_id)
            .action(AuditAction::SessionStarted {
                user_id,
                platform: "cli".to_string(),
            })
            .authorization(AuthorizationProof::System {
                reason: "test".to_string(),
            })
            .success()
            .unwrap();

        assert!(log.get(&entry_id).unwrap().is_some());
    }

    #[test]
    fn test_verify_detects_tampered_signature() {
        let keypair = KeyPair::generate();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();
        let ids = append_test_entries(&log, &session_id, 3);

        // Tamper: corrupt the signature of the second entry.
        let mut entry = log.get(&ids[1]).unwrap().unwrap();
        let mut bad_sig = *entry.signature.as_bytes();
        bad_sig[0] ^= 0xFF;
        entry.signature = astrid_crypto::Signature::from_bytes(bad_sig);
        log.storage.store(&entry).unwrap();

        let result = log.verify_chain(&session_id).unwrap();
        assert!(!result.valid);
        assert!(result.issues.iter().any(|issue| matches!(
            issue,
            ChainIssue::InvalidSignature { entry_id } if *entry_id == ids[1]
        )));
    }

    #[test]
    fn test_verify_detects_broken_link() {
        let keypair = KeyPair::generate();
        // Keep secret bytes to reconstruct the key for re-signing tampered entries.
        let secret = keypair.secret_key_bytes();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();
        let ids = append_test_entries(&log, &session_id, 3);

        // Tamper: change the previous_hash of the third entry to break the link.
        let mut entry = log.get(&ids[2]).unwrap().unwrap();
        entry.previous_hash = ContentHash::from_bytes([0xAB; 32]);
        // Re-sign so the signature is valid - only the link is broken.
        let signer = KeyPair::from_secret_key(&secret).unwrap();
        let signing_data = entry.signing_data();
        entry.signature = signer.sign(&signing_data);
        log.storage.store(&entry).unwrap();

        let result = log.verify_chain(&session_id).unwrap();
        assert!(!result.valid);
        // The re-sign must succeed - no InvalidSignature, only BrokenLink.
        assert!(
            !result
                .issues
                .iter()
                .any(|issue| matches!(issue, ChainIssue::InvalidSignature { .. })),
            "re-signed entry should not trigger InvalidSignature"
        );
        assert!(result.issues.iter().any(|issue| matches!(
            issue,
            ChainIssue::BrokenLink { entry_id, .. } if *entry_id == ids[2]
        )));
    }

    #[test]
    fn test_verify_detects_invalid_genesis() {
        let keypair = KeyPair::generate();
        let secret = keypair.secret_key_bytes();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();

        // Create one entry then tamper its previous_hash to be non-zero.
        let id = log
            .append(
                session_id.clone(),
                AuditAction::McpToolCall {
                    server: "test".to_string(),
                    tool: "tool_0".to_string(),
                    args_hash: ContentHash::zero(),
                },
                AuthorizationProof::NotRequired {
                    reason: "test".to_string(),
                },
                AuditOutcome::success(),
            )
            .unwrap();

        let mut entry = log.get(&id).unwrap().unwrap();
        entry.previous_hash = ContentHash::from_bytes([0x01; 32]);
        // Re-sign with the tampered previous_hash.
        let signer = KeyPair::from_secret_key(&secret).unwrap();
        let signing_data = entry.signing_data();
        entry.signature = signer.sign(&signing_data);
        log.storage.store(&entry).unwrap();

        let result = log.verify_chain(&session_id).unwrap();
        assert!(!result.valid);
        // The re-sign must succeed - no InvalidSignature, only InvalidGenesis.
        assert!(
            !result
                .issues
                .iter()
                .any(|issue| matches!(issue, ChainIssue::InvalidSignature { .. })),
            "re-signed entry should not trigger InvalidSignature"
        );
        assert!(result.issues.iter().any(|issue| matches!(
            issue,
            ChainIssue::InvalidGenesis { entry_id } if *entry_id == id
        )));
    }

    #[test]
    fn test_verify_all_detects_tampered_session() {
        let keypair = KeyPair::generate();
        let log = AuditLog::in_memory(keypair);

        // Session A: valid chain.
        let session_a = SessionId::new();
        append_test_entries(&log, &session_a, 3);

        // Session B: tampered chain (single entry).
        let session_b = SessionId::new();
        let tampered_ids = append_test_entries(&log, &session_b, 1);
        let tampered_id = tampered_ids[0].clone();

        // Corrupt session B's entry signature.
        let mut entry = log.get(&tampered_id).unwrap().unwrap();
        let mut bad_sig = *entry.signature.as_bytes();
        bad_sig[0] ^= 0xFF;
        entry.signature = astrid_crypto::Signature::from_bytes(bad_sig);
        log.storage.store(&entry).unwrap();

        let results = log.verify_all().unwrap();
        assert_eq!(results.len(), 2);

        let a_result = results.iter().find(|(sid, _)| *sid == session_a).unwrap();
        assert!(a_result.1.valid);

        let b_result = results.iter().find(|(sid, _)| *sid == session_b).unwrap();
        assert!(!b_result.1.valid);
    }

    #[test]
    fn test_verify_empty_log_is_valid() {
        let keypair = KeyPair::generate();
        let log = AuditLog::in_memory(keypair);

        let results = log.verify_all().unwrap();
        assert!(results.is_empty());

        // Also verify an empty session.
        let session_id = SessionId::new();
        let result = log.verify_chain(&session_id).unwrap();
        assert!(result.valid);
        assert_eq!(result.entries_verified, 0);
    }

    #[test]
    fn test_key_rotation_entries_verify_via_embedded_pubkey() {
        // Entries embed the public key they were signed with, so verification
        // works even when the log's runtime key has changed (key rotation).
        let keypair_a = KeyPair::generate();
        let log_a = AuditLog::in_memory(keypair_a);
        let session_id = SessionId::new();

        // Write entries signed by key A.
        append_test_entries(&log_a, &session_id, 3);

        // Extract the entries and replay them into a log with key B.
        let entries = log_a.get_session_entries(&session_id).unwrap();
        let keypair_b = KeyPair::generate();
        let log_b = AuditLog::in_memory(keypair_b);

        for entry in &entries {
            log_b.storage.store(entry).unwrap();
        }

        // Key B log should still verify entries signed by key A because
        // verify_signature uses the entry's embedded public key.
        let result = log_b.verify_chain(&session_id).unwrap();
        assert!(
            result.valid,
            "entries signed by key A should verify in key B log, issues: {:?}",
            result.issues
        );
        assert_eq!(result.entries_verified, 3);
    }

    // ── Per-principal chain tests ────────────────────────────────

    #[test]
    fn test_principal_chains_are_independent() {
        let keypair = KeyPair::generate();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();
        let alice = astrid_core::PrincipalId::new("alice").unwrap();
        let bob = astrid_core::PrincipalId::new("bob").unwrap();

        // Alice: 2 entries
        log.append_with_principal(
            session_id.clone(),
            alice.clone(),
            AuditAction::McpToolCall {
                server: "test".into(),
                tool: "alice_tool_1".into(),
                args_hash: ContentHash::zero(),
            },
            AuthorizationProof::NotRequired {
                reason: "test".into(),
            },
            AuditOutcome::success(),
        )
        .unwrap();
        log.append_with_principal(
            session_id.clone(),
            alice.clone(),
            AuditAction::McpToolCall {
                server: "test".into(),
                tool: "alice_tool_2".into(),
                args_hash: ContentHash::zero(),
            },
            AuthorizationProof::NotRequired {
                reason: "test".into(),
            },
            AuditOutcome::success(),
        )
        .unwrap();

        // Bob: 1 entry
        log.append_with_principal(
            session_id.clone(),
            bob.clone(),
            AuditAction::McpToolCall {
                server: "test".into(),
                tool: "bob_tool_1".into(),
                args_hash: ContentHash::zero(),
            },
            AuthorizationProof::NotRequired {
                reason: "test".into(),
            },
            AuditOutcome::success(),
        )
        .unwrap();

        // System: 1 entry
        log.append(
            session_id.clone(),
            AuditAction::SessionStarted {
                user_id: [0; 8],
                platform: "test".into(),
            },
            AuthorizationProof::System {
                reason: "test".into(),
            },
            AuditOutcome::success(),
        )
        .unwrap();

        // Each chain verifies independently.
        let alice_result = log
            .verify_principal_chain(&session_id, Some(&alice))
            .unwrap();
        assert!(alice_result.valid, "alice chain: {:?}", alice_result.issues);
        assert_eq!(alice_result.entries_verified, 2);

        let bob_result = log.verify_principal_chain(&session_id, Some(&bob)).unwrap();
        assert!(bob_result.valid, "bob chain: {:?}", bob_result.issues);
        assert_eq!(bob_result.entries_verified, 1);

        let system_result = log.verify_principal_chain(&session_id, None).unwrap();
        assert!(
            system_result.valid,
            "system chain: {:?}",
            system_result.issues
        );
        assert_eq!(system_result.entries_verified, 1);

        // Full session verification covers all 4 entries.
        let full = log.verify_chain(&session_id).unwrap();
        assert!(full.valid, "full session: {:?}", full.issues);
        assert_eq!(full.entries_verified, 4);
    }

    #[test]
    fn test_get_principal_entries_filters_correctly() {
        let keypair = KeyPair::generate();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();
        let alice = astrid_core::PrincipalId::new("alice").unwrap();

        // 2 alice entries + 1 system entry
        log.append_with_principal(
            session_id.clone(),
            alice.clone(),
            AuditAction::FileRead {
                path: "a.txt".into(),
            },
            AuthorizationProof::NotRequired { reason: "t".into() },
            AuditOutcome::success(),
        )
        .unwrap();
        log.append(
            session_id.clone(),
            AuditAction::ConfigReloaded,
            AuthorizationProof::System { reason: "t".into() },
            AuditOutcome::success(),
        )
        .unwrap();
        log.append_with_principal(
            session_id.clone(),
            alice.clone(),
            AuditAction::FileRead {
                path: "b.txt".into(),
            },
            AuthorizationProof::NotRequired { reason: "t".into() },
            AuditOutcome::success(),
        )
        .unwrap();

        let alice_entries = log
            .get_principal_entries(&session_id, Some(&alice))
            .unwrap();
        assert_eq!(alice_entries.len(), 2);

        let system_entries = log.get_principal_entries(&session_id, None).unwrap();
        assert_eq!(system_entries.len(), 1);

        // Total session still has 3
        let all = log.get_session_entries(&session_id).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_mixed_session_verify_chain_passes() {
        // A session with interleaved principal and system entries
        // should verify cleanly — each chain is independent.
        let keypair = KeyPair::generate();
        let log = AuditLog::in_memory(keypair);
        let session_id = SessionId::new();
        let alice = astrid_core::PrincipalId::new("alice").unwrap();

        // Interleave: system, alice, system, alice
        log.append(
            session_id.clone(),
            AuditAction::ConfigReloaded,
            AuthorizationProof::System { reason: "t".into() },
            AuditOutcome::success(),
        )
        .unwrap();
        log.append_with_principal(
            session_id.clone(),
            alice.clone(),
            AuditAction::FileRead {
                path: "a.txt".into(),
            },
            AuthorizationProof::NotRequired { reason: "t".into() },
            AuditOutcome::success(),
        )
        .unwrap();
        log.append(
            session_id.clone(),
            AuditAction::ConfigReloaded,
            AuthorizationProof::System { reason: "t".into() },
            AuditOutcome::success(),
        )
        .unwrap();
        log.append_with_principal(
            session_id.clone(),
            alice.clone(),
            AuditAction::FileRead {
                path: "b.txt".into(),
            },
            AuthorizationProof::NotRequired { reason: "t".into() },
            AuditOutcome::success(),
        )
        .unwrap();

        let result = log.verify_chain(&session_id).unwrap();
        assert!(result.valid, "mixed chain: {:?}", result.issues);
        assert_eq!(result.entries_verified, 4);
    }
}
