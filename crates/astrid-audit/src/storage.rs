//! Audit log storage trait and SurrealKV-based implementation.

use astrid_capabilities::AuditEntryId;
use astrid_core::SessionId;
use astrid_storage::{KvStore, MemoryKvStore, SurrealKvStore};
use std::path::Path;
use std::sync::Arc;

use crate::entry::AuditEntry;
use crate::error::{AuditError, AuditResult};

/// Storage backend for audit logs.
///
/// Implementations must be thread-safe and support:
/// - Storing and retrieving individual entries
/// - Session-scoped queries
/// - Chain head tracking (latest entry per session)
pub trait AuditStorage: Send + Sync {
    /// Store an audit entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry cannot be persisted.
    fn store(&self, entry: &AuditEntry) -> AuditResult<()>;

    /// Get an entry by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval or deserialization fails.
    fn get(&self, id: &AuditEntryId) -> AuditResult<Option<AuditEntry>>;

    /// Get the chain head (latest entry ID) for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval or parsing fails.
    fn get_chain_head(&self, session_id: &SessionId) -> AuditResult<Option<AuditEntryId>>;

    /// Get all entries for a session, in insertion order.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval or deserialization fails.
    fn get_session_entries(&self, session_id: &SessionId) -> AuditResult<Vec<AuditEntry>>;

    /// Get entries in a time range.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval or deserialization fails.
    fn get_entries_in_range(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> AuditResult<Vec<AuditEntry>>;

    /// Count total entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails.
    fn count(&self) -> AuditResult<usize>;

    /// Count entries for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval or deserialization fails.
    fn count_session(&self, session_id: &SessionId) -> AuditResult<usize>;

    /// List all session IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval or parsing fails.
    fn list_sessions(&self) -> AuditResult<Vec<SessionId>>;

    /// Flush pending writes to durable storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend fails to flush.
    fn flush(&self) -> AuditResult<()>;
}

// -- Namespace constants --

const NS_ENTRIES: &str = "audit:entries";
const NS_SESSION_INDEX: &str = "audit:session_index";
const NS_CHAIN_HEADS: &str = "audit:chain_heads";

/// Run an async future synchronously.
///
/// `SurrealKV` operations are fast in-process (no network), so bridging
/// the sync `AuditStorage` trait to the async `KvStore` trait is safe.
///
/// Handles three cases:
/// - Inside an async context: uses a scoped thread to avoid the
///   "cannot `block_on` from within a runtime" panic.
/// - Outside a runtime: creates a temporary runtime.
fn block_on<F>(f: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // Inside a tokio runtime — use a scoped thread to avoid panic.
            std::thread::scope(|s| s.spawn(|| handle.block_on(f)).join().unwrap())
        },
        Err(_) => {
            // No runtime — create a lightweight current-thread runtime.
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime")
                .block_on(f)
        },
    }
}

/// SurrealKV-based storage backend for audit logs.
pub struct SurrealKvAuditStorage {
    store: Arc<dyn KvStore>,
}

impl SurrealKvAuditStorage {
    /// Open or create audit storage at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the `SurrealKV` store fails to open.
    pub fn open(path: impl AsRef<Path>) -> AuditResult<Self> {
        let store =
            SurrealKvStore::open(path).map_err(|e| AuditError::StorageError(e.to_string()))?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Create an in-memory storage (for testing).
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            store: Arc::new(MemoryKvStore::new()),
        }
    }

    /// Get all entry IDs for a session (from the session index).
    fn get_session_entry_ids(&self, session_id: &SessionId) -> AuditResult<Vec<AuditEntryId>> {
        let key = session_id.0.to_string();

        let data = block_on(self.store.get(NS_SESSION_INDEX, &key))
            .map_err(|e| AuditError::StorageError(e.to_string()))?;

        match data {
            Some(bytes) => {
                let ids: Vec<AuditEntryId> = serde_json::from_slice(&bytes)
                    .map_err(|e| AuditError::SerializationError(e.to_string()))?;
                Ok(ids)
            },
            None => Ok(Vec::new()),
        }
    }
}

impl AuditStorage for SurrealKvAuditStorage {
    fn store(&self, entry: &AuditEntry) -> AuditResult<()> {
        let entry_key = entry.id.0.to_string();
        let session_key = entry.session_id.0.to_string();

        // Serialize entry.
        let entry_data =
            serde_json::to_vec(entry).map_err(|e| AuditError::SerializationError(e.to_string()))?;

        // Store entry.
        block_on(self.store.set(NS_ENTRIES, &entry_key, entry_data))
            .map_err(|e| AuditError::StorageError(e.to_string()))?;

        // Update session index (append entry ID to the list).
        let mut entry_ids = self.get_session_entry_ids(&entry.session_id)?;
        entry_ids.push(entry.id.clone());
        let index_data = serde_json::to_vec(&entry_ids)
            .map_err(|e| AuditError::SerializationError(e.to_string()))?;
        block_on(self.store.set(NS_SESSION_INDEX, &session_key, index_data))
            .map_err(|e| AuditError::StorageError(e.to_string()))?;

        // Update chain head.
        block_on(
            self.store
                .set(NS_CHAIN_HEADS, &session_key, entry_key.into_bytes()),
        )
        .map_err(|e| AuditError::StorageError(e.to_string()))?;

        Ok(())
    }

    fn get(&self, id: &AuditEntryId) -> AuditResult<Option<AuditEntry>> {
        let key = id.0.to_string();

        let data = block_on(self.store.get(NS_ENTRIES, &key))
            .map_err(|e| AuditError::StorageError(e.to_string()))?;

        match data {
            Some(bytes) => {
                let entry = serde_json::from_slice(&bytes)
                    .map_err(|e| AuditError::SerializationError(e.to_string()))?;
                Ok(Some(entry))
            },
            None => Ok(None),
        }
    }

    fn get_chain_head(&self, session_id: &SessionId) -> AuditResult<Option<AuditEntryId>> {
        let key = session_id.0.to_string();

        let data = block_on(self.store.get(NS_CHAIN_HEADS, &key))
            .map_err(|e| AuditError::StorageError(e.to_string()))?;

        match data {
            Some(bytes) => {
                let id_str = std::str::from_utf8(&bytes)
                    .map_err(|e| AuditError::StorageError(e.to_string()))?;
                let uuid = uuid::Uuid::parse_str(id_str)
                    .map_err(|e| AuditError::StorageError(e.to_string()))?;
                Ok(Some(AuditEntryId(uuid)))
            },
            None => Ok(None),
        }
    }

    fn get_session_entries(&self, session_id: &SessionId) -> AuditResult<Vec<AuditEntry>> {
        let ids = self.get_session_entry_ids(session_id)?;
        let mut entries = Vec::with_capacity(ids.len());

        for id in ids {
            if let Some(entry) = self.get(&id)? {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    fn get_entries_in_range(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> AuditResult<Vec<AuditEntry>> {
        // List all sessions, then filter entries by timestamp.
        let sessions = self.list_sessions()?;
        let mut entries = Vec::new();

        for session_id in sessions {
            for entry in self.get_session_entries(&session_id)? {
                let ts = entry.timestamp.0;
                if ts >= start && ts <= end {
                    entries.push(entry);
                }
            }
        }

        // Sort by timestamp.
        entries.sort_by(|a, b| a.timestamp.0.cmp(&b.timestamp.0));

        Ok(entries)
    }

    fn count(&self) -> AuditResult<usize> {
        let keys = block_on(self.store.list_keys(NS_ENTRIES))
            .map_err(|e| AuditError::StorageError(e.to_string()))?;
        Ok(keys.len())
    }

    fn count_session(&self, session_id: &SessionId) -> AuditResult<usize> {
        Ok(self.get_session_entry_ids(session_id)?.len())
    }

    fn list_sessions(&self) -> AuditResult<Vec<SessionId>> {
        let keys = block_on(self.store.list_keys(NS_SESSION_INDEX))
            .map_err(|e| AuditError::StorageError(e.to_string()))?;

        let mut sessions = Vec::new();
        for key in keys {
            if let Ok(uuid) = uuid::Uuid::parse_str(&key) {
                sessions.push(SessionId::from_uuid(uuid));
            }
        }

        Ok(sessions)
    }

    fn flush(&self) -> AuditResult<()> {
        // KvStore commits on every set(), no explicit flush needed.
        Ok(())
    }
}

impl std::fmt::Debug for SurrealKvAuditStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurrealKvAuditStorage")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{AuditAction, AuditOutcome, AuthorizationProof};
    use astrid_crypto::{ContentHash, KeyPair};

    fn test_keypair() -> KeyPair {
        KeyPair::generate()
    }

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let storage = SurrealKvAuditStorage::in_memory();
        let keypair = test_keypair();
        let session_id = SessionId::new();

        let entry = AuditEntry::create(
            session_id.clone(),
            AuditAction::SessionStarted {
                user_id: keypair.key_id(),
                frontend: "cli".to_string(),
            },
            AuthorizationProof::System {
                reason: "test".to_string(),
            },
            AuditOutcome::success(),
            ContentHash::zero(),
            &keypair,
        );

        let entry_id = entry.id.clone();

        storage.store(&entry).unwrap();

        let retrieved = storage.get(&entry_id).unwrap().unwrap();
        assert_eq!(retrieved.id, entry_id);
    }

    #[tokio::test]
    async fn test_session_index() {
        let storage = SurrealKvAuditStorage::in_memory();
        let keypair = test_keypair();
        let session_id = SessionId::new();

        // Create multiple entries
        let mut prev_hash = ContentHash::zero();
        for i in 0..3 {
            let entry = AuditEntry::create(
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
                prev_hash,
                &keypair,
            );
            prev_hash = entry.content_hash();
            storage.store(&entry).unwrap();
        }

        let entries = storage.get_session_entries(&session_id).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn test_chain_head() {
        let storage = SurrealKvAuditStorage::in_memory();
        let keypair = test_keypair();
        let session_id = SessionId::new();

        let entry1 = AuditEntry::create(
            session_id.clone(),
            AuditAction::SessionStarted {
                user_id: keypair.key_id(),
                frontend: "cli".to_string(),
            },
            AuthorizationProof::System {
                reason: "test".to_string(),
            },
            AuditOutcome::success(),
            ContentHash::zero(),
            &keypair,
        );

        storage.store(&entry1).unwrap();

        let entry2 = AuditEntry::create(
            session_id.clone(),
            AuditAction::SessionEnded {
                reason: "done".to_string(),
                duration_secs: 100,
            },
            AuthorizationProof::System {
                reason: "test".to_string(),
            },
            AuditOutcome::success(),
            entry1.content_hash(),
            &keypair,
        );

        storage.store(&entry2).unwrap();

        let head = storage.get_chain_head(&session_id).unwrap().unwrap();
        assert_eq!(head, entry2.id);
    }
}
