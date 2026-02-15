//! Session persistence.
//!
//! Stores and retrieves sessions from disk. Sessions live in
//! `~/.astralis/sessions/` (the global home directory) and are linked to
//! workspaces via workspace IDs stored in each session's JSON.
//!
//! # Crash Safety
//!
//! Writes use atomic write-to-tempfile + rename to prevent corruption if the
//! process crashes mid-write.

use astralis_core::SessionId;
use astralis_core::dirs::AstralisHome;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use crate::error::{RuntimeError, RuntimeResult};
use crate::session::{AgentSession, SerializableSession};

/// Session store for persistence.
///
/// Directory creation is lazy — the sessions directory is only created on
/// the first call to [`save`](Self::save), not at construction time.
pub struct SessionStore {
    /// Directory for session files.
    sessions_dir: PathBuf,
    /// Whether the directory has been ensured to exist.
    dir_ensured: std::sync::atomic::AtomicBool,
}

impl SessionStore {
    /// Create a new session store pointing at an explicit directory.
    ///
    /// The directory is **not** created immediately — it will be created
    /// lazily on the first save.
    #[must_use]
    pub fn new(sessions_dir: impl AsRef<Path>) -> Self {
        let sessions_dir = sessions_dir.as_ref().to_path_buf();
        let dir_exists = sessions_dir.is_dir();
        Self {
            sessions_dir,
            dir_ensured: std::sync::atomic::AtomicBool::new(dir_exists),
        }
    }

    /// Create a session store from an [`AstralisHome`].
    ///
    /// Sessions will be stored in `~/.astralis/sessions/`.
    /// The directory is created lazily on first save.
    #[must_use]
    pub fn from_home(home: &AstralisHome) -> Self {
        Self::new(home.sessions_dir())
    }

    /// Ensure the sessions directory exists (called lazily on first write).
    fn ensure_dir(&self) -> RuntimeResult<()> {
        if self.dir_ensured.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        std::fs::create_dir_all(&self.sessions_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Ensure the sessions dir and its parent (.astralis/) are owner-only
            let perms = std::fs::Permissions::from_mode(0o700);
            if let Some(parent) = self.sessions_dir.parent() {
                let _ = std::fs::set_permissions(parent, perms.clone());
            }
            let _ = std::fs::set_permissions(&self.sessions_dir, perms);
        }
        self.dir_ensured
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Get the path for a session file.
    fn session_path(&self, id: &SessionId) -> PathBuf {
        self.sessions_dir.join(format!("{}.json", id.0))
    }

    /// Save a session atomically.
    ///
    /// Writes to a temporary file first, then renames. This prevents corruption
    /// if the process crashes mid-write (session auto-saves after every turn).
    ///
    /// # Errors
    ///
    /// Returns an error if the session cannot be serialized or written to disk.
    pub fn save(&self, session: &AgentSession) -> RuntimeResult<()> {
        self.ensure_dir()?;

        let path = self.session_path(&session.id);
        let serializable = SerializableSession::from(session);

        let json = serde_json::to_string_pretty(&serializable)
            .map_err(|e| RuntimeError::SerializationError(e.to_string()))?;

        // Atomic write: write to temp file, then rename
        let temp_path = path.with_extension("json.tmp");
        std::fs::write(&temp_path, &json)?;
        std::fs::rename(&temp_path, &path).inspect_err(|_| {
            // Clean up temp file on rename failure
            let _ = std::fs::remove_file(&temp_path);
        })?;

        debug!(session_id = %session.id, path = ?path, "Session saved");

        Ok(())
    }

    /// Load a session by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the session file cannot be read or deserialized.
    pub fn load(&self, id: &SessionId) -> RuntimeResult<Option<AgentSession>> {
        let path = self.session_path(id);

        if !path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&path)?;
        let serializable: SerializableSession = serde_json::from_str(&json)
            .map_err(|e| RuntimeError::SerializationError(e.to_string()))?;

        let session = serializable.to_session();

        debug!(session_id = %id, "Session loaded");

        Ok(Some(session))
    }

    /// Load a session by ID string.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is not a valid UUID or the session cannot be loaded.
    pub fn load_by_str(&self, id: &str) -> RuntimeResult<Option<AgentSession>> {
        let uuid =
            uuid::Uuid::parse_str(id).map_err(|e| RuntimeError::StorageError(e.to_string()))?;
        self.load(&SessionId::from_uuid(uuid))
    }

    /// Delete a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the session file cannot be deleted.
    pub fn delete(&self, id: &SessionId) -> RuntimeResult<()> {
        let path = self.session_path(id);

        if path.exists() {
            std::fs::remove_file(&path)?;
            info!(session_id = %id, "Session deleted");
        }

        Ok(())
    }

    /// List all session IDs, sorted by modification time (most recent first).
    ///
    /// Returns an empty list if the sessions directory does not exist yet.
    ///
    /// # Errors
    ///
    /// Returns an error if the sessions directory cannot be read.
    pub fn list(&self) -> RuntimeResult<Vec<SessionId>> {
        if !self.sessions_dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();

        for entry in std::fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|e| e == "json")
                && let Some(stem) = path.file_stem()
                && let Some(stem_str) = stem.to_str()
                && let Ok(uuid) = uuid::Uuid::parse_str(stem_str)
            {
                sessions.push(SessionId::from_uuid(uuid));
            }
        }

        // Sort by modification time (most recent first)
        sessions.sort_by(|a, b| {
            let path_a = self.session_path(a);
            let path_b = self.session_path(b);

            let time_a = std::fs::metadata(&path_a)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let time_b = std::fs::metadata(&path_b)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

            time_b.cmp(&time_a)
        });

        Ok(sessions)
    }

    /// List sessions with metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the sessions directory cannot be read.
    pub fn list_with_metadata(&self) -> RuntimeResult<Vec<SessionSummary>> {
        let ids = self.list()?;
        let mut summaries = Vec::new();

        for id in ids {
            if let Ok(Some(session)) = self.load(&id) {
                summaries.push(SessionSummary {
                    id: id.0.to_string(),
                    title: session.metadata.title.clone(),
                    created_at: session.created_at,
                    message_count: session.messages.len(),
                    token_count: session.token_count,
                    workspace_path: session.workspace_path.clone(),
                });
            }
        }

        Ok(summaries)
    }

    /// Get the most recent session.
    ///
    /// # Errors
    ///
    /// Returns an error if the sessions cannot be listed or loaded.
    pub fn most_recent(&self) -> RuntimeResult<Option<AgentSession>> {
        let ids = self.list()?;
        if let Some(id) = ids.first() {
            self.load(id)
        } else {
            Ok(None)
        }
    }

    /// List sessions filtered by workspace path.
    ///
    /// Only returns sessions whose `workspace_path` matches the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if sessions cannot be listed or loaded.
    pub fn list_for_workspace(&self, workspace: &Path) -> RuntimeResult<Vec<SessionSummary>> {
        let all = self.list_with_metadata()?;
        Ok(all
            .into_iter()
            .filter(|s| s.workspace_path.as_deref().is_some_and(|p| p == workspace))
            .collect())
    }

    /// Clean up old sessions (older than N days).
    ///
    /// # Errors
    ///
    /// Returns an error if the sessions cannot be listed.
    pub fn cleanup_old(&self, max_age_days: i64) -> RuntimeResult<usize> {
        // Safety: subtracting a known-positive duration from current time
        #[allow(clippy::arithmetic_side_effects)]
        let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days);
        let mut removed = 0usize;

        for id in self.list()? {
            if let Ok(Some(session)) = self.load(&id)
                && session.created_at < cutoff
                && self.delete(&id).is_ok()
            {
                removed = removed.saturating_add(1);
            }
        }

        Ok(removed)
    }
}

/// Summary of a session for listing.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    /// Session ID.
    pub id: String,
    /// Session title.
    pub title: Option<String>,
    /// Created timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Number of messages.
    pub message_count: usize,
    /// Token count.
    pub token_count: usize,
    /// Workspace path (for workspace-scoped listing).
    pub workspace_path: Option<PathBuf>,
}

impl SessionSummary {
    /// Get a display title.
    #[must_use]
    pub fn display_title(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            let short_id = &self.id[..8];
            format!("Session {short_id}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_store() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp_dir.path());

        let session = AgentSession::new([0u8; 8], "Test");

        // Save (lazily creates dir)
        store.save(&session).unwrap();

        // Load
        let loaded = store.load(&session.id).unwrap().unwrap();
        assert_eq!(loaded.system_prompt, session.system_prompt);

        // List
        let ids = store.list().unwrap();
        assert_eq!(ids.len(), 1);

        // Delete
        store.delete(&session.id).unwrap();
        assert!(store.load(&session.id).unwrap().is_none());
    }

    #[test]
    fn test_session_store_lazy_dir_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sessions_path = temp_dir.path().join("lazy_sessions");

        let store = SessionStore::new(&sessions_path);

        // Directory should not exist yet
        assert!(!sessions_path.exists());

        // List on non-existent dir returns empty
        let ids = store.list().unwrap();
        assert!(ids.is_empty());

        // Save creates the directory
        let session = AgentSession::new([0u8; 8], "Test");
        store.save(&session).unwrap();
        assert!(sessions_path.exists());
    }

    #[test]
    fn test_session_store_atomic_write() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp_dir.path());

        let session = AgentSession::new([0u8; 8], "Test");
        store.save(&session).unwrap();

        // No temp file should remain
        let temp_path = temp_dir.path().join(format!("{}.json.tmp", session.id.0));
        assert!(!temp_path.exists());

        // The real file should exist
        let real_path = temp_dir.path().join(format!("{}.json", session.id.0));
        assert!(real_path.exists());
    }

    #[test]
    fn test_session_store_from_home() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home = AstralisHome::from_path(temp_dir.path());
        let store = SessionStore::from_home(&home);

        let session = AgentSession::new([0u8; 8], "Test");
        store.save(&session).unwrap();

        // Should be saved under sessions/
        let expected = temp_dir
            .path()
            .join("sessions")
            .join(format!("{}.json", session.id.0));
        assert!(expected.exists());
    }
}
