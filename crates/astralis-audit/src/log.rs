//! Audit log - main interface for audit logging.
//!
//! Provides a high-level API for recording and verifying audit entries.

use astralis_capabilities::AuditEntryId;
use astralis_core::SessionId;
use astralis_crypto::{ContentHash, KeyPair};
use std::path::Path;
use std::sync::RwLock;
use tracing::{debug, error, warn};

use crate::entry::{AuditAction, AuditEntry, AuditOutcome, AuthorizationProof};
use crate::error::{AuditError, AuditResult};
use crate::storage::{AuditStorage, SurrealKvAuditStorage};

/// Audit log for recording and verifying security events.
pub struct AuditLog {
    /// Storage backend.
    storage: Box<dyn AuditStorage>,
    /// Runtime signing key.
    runtime_key: KeyPair,
    /// Current chain heads per session (cached for performance).
    chain_heads: RwLock<std::collections::HashMap<SessionId, ContentHash>>,
}

impl AuditLog {
    /// Create a new audit log with a custom storage backend.
    #[must_use]
    pub fn with_storage(storage: Box<dyn AuditStorage>, runtime_key: KeyPair) -> Self {
        Self {
            storage,
            runtime_key,
            chain_heads: RwLock::new(std::collections::HashMap::new()),
        }
    }

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
        // Get the previous hash for this session
        let previous_hash = self.get_previous_hash(&session_id)?;

        // Create and sign the entry
        let entry = AuditEntry::create(
            session_id.clone(),
            action,
            authorization,
            outcome,
            previous_hash,
            &self.runtime_key,
        );

        let entry_id = entry.id.clone();
        let entry_hash = entry.content_hash();

        debug!(
            entry_id = %entry_id,
            action = %entry.action.description(),
            "Appending audit entry"
        );

        // Store the entry
        self.storage.store(&entry)?;

        // Update cached chain head
        {
            let mut heads = self
                .chain_heads
                .write()
                .map_err(|e| AuditError::StorageError(e.to_string()))?;
            heads.insert(session_id, entry_hash);
        }

        Ok(entry_id)
    }

    /// Get the previous hash for a session (for chain linking).
    fn get_previous_hash(&self, session_id: &SessionId) -> AuditResult<ContentHash> {
        // Check cache first
        {
            let heads = self
                .chain_heads
                .read()
                .map_err(|e| AuditError::StorageError(e.to_string()))?;
            if let Some(hash) = heads.get(session_id) {
                return Ok(*hash);
            }
        }

        // Check storage
        if let Some(head_id) = self.storage.get_chain_head(session_id)?
            && let Some(entry) = self.storage.get(&head_id)?
        {
            return Ok(entry.content_hash());
        }

        // Genesis - no previous entry
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

    /// Verify the integrity of the audit chain for a session.
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

        let mut issues = Vec::new();
        let mut entries_verified: usize = 0;

        // Sort by timestamp to ensure correct order
        let mut sorted_entries = entries;
        sorted_entries.sort_by(|a, b| a.timestamp.0.cmp(&b.timestamp.0));

        // Verify first entry has zero previous hash
        if !sorted_entries[0].previous_hash.is_zero() {
            issues.push(ChainIssue::InvalidGenesis {
                entry_id: sorted_entries[0].id.clone(),
            });
        }

        // Verify signatures
        for entry in &sorted_entries {
            if let Err(e) = entry.verify_signature() {
                error!(entry_id = %entry.id, error = %e, "Invalid signature");
                issues.push(ChainIssue::InvalidSignature {
                    entry_id: entry.id.clone(),
                });
            }
            entries_verified = entries_verified.saturating_add(1);
        }

        // Verify chain linking
        for i in 1..sorted_entries.len() {
            // Safety: i starts at 1, so i-1 is always valid
            #[allow(clippy::arithmetic_side_effects)]
            let prev = &sorted_entries[i - 1];
            let curr = &sorted_entries[i];

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

        Ok(ChainVerificationResult {
            valid: issues.is_empty(),
            entries_verified,
            issues,
        })
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
    pub fn runtime_public_key(&self) -> astralis_crypto::PublicKey {
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
pub struct AuditBuilder<'a> {
    log: &'a AuditLog,
    session_id: SessionId,
    action: Option<AuditAction>,
    authorization: Option<AuthorizationProof>,
}

impl<'a> AuditBuilder<'a> {
    /// Create a new audit builder.
    pub fn new(log: &'a AuditLog, session_id: SessionId) -> Self {
        Self {
            log,
            session_id,
            action: None,
            authorization: None,
        }
    }

    /// Set the action.
    #[must_use]
    pub fn action(mut self, action: AuditAction) -> Self {
        self.action = Some(action);
        self
    }

    /// Set the authorization.
    #[must_use]
    pub fn authorization(mut self, auth: AuthorizationProof) -> Self {
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
    pub fn success(self) -> AuditResult<AuditEntryId> {
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
    pub fn success_with(self, details: impl Into<String>) -> AuditResult<AuditEntryId> {
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
    pub fn failure(self, error: impl Into<String>) -> AuditResult<AuditEntryId> {
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
                    frontend: "cli".to_string(),
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

        // Create a chain of entries
        for i in 0..5 {
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
            .unwrap();
        }

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
                frontend: "cli".to_string(),
            })
            .authorization(AuthorizationProof::System {
                reason: "test".to_string(),
            })
            .success()
            .unwrap();

        assert!(log.get(&entry_id).unwrap().is_some());
    }
}
