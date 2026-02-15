//! Plugin lockfile for version pinning and integrity verification.
//!
//! The lockfile (`.astrid/plugins.lock` at workspace level,
//! `~/.astrid/plugins.lock` at user level) tracks exactly
//! what was installed, from where, and its integrity hash. This
//! enables reproducible builds and supply chain auditing.
//!
//! # Format
//!
//! The lockfile uses TOML with `schema_version = 1` and a flat
//! `[[plugin]]` array of [`LockedPlugin`] entries.

use std::fmt;
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::discovery::MANIFEST_FILE_NAME;
use crate::error::{PluginError, PluginResult};
use crate::manifest::PluginManifest;
use crate::plugin::PluginId;

/// Current lockfile schema version.
const SCHEMA_VERSION: u32 = 1;

/// Standard lockfile file name.
pub const LOCKFILE_NAME: &str = "plugins.lock";

/// A plugin lockfile that tracks installed plugins, their sources,
/// and integrity hashes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginLockfile {
    /// Schema version for forward compatibility.
    schema_version: u32,
    /// Locked plugin entries.
    #[serde(default, rename = "plugin")]
    entries: Vec<LockedPlugin>,
}

/// A single locked plugin entry recording what was installed and
/// its cryptographic integrity hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedPlugin {
    /// Unique plugin identifier.
    pub id: PluginId,
    /// Semantic version string from the manifest at install time.
    pub version: String,
    /// Where the plugin was installed from.
    pub source: PluginSource,
    /// Blake3 hex digest of the WASM module (prefixed with `blake3:`).
    pub wasm_hash: String,
    /// When the plugin was installed/last updated.
    pub installed_at: DateTime<Utc>,
}

/// Where a plugin was installed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum PluginSource {
    /// Installed from a local directory path.
    Local(String),
    /// Fetched from the `OpenClaw` npm registry.
    OpenClaw(String),
    /// Fetched from a git repository (URL + optional commit).
    Git {
        /// Repository URL.
        url: String,
        /// Commit hash, if pinned.
        commit: Option<String>,
    },
    /// Fetched from the Astrid plugin registry.
    Registry(String),
}

impl fmt::Display for PluginSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(path) => write!(f, "local:{path}"),
            Self::OpenClaw(spec) => write!(f, "openclaw:{spec}"),
            Self::Git { url, commit: None } => write!(f, "git:{url}"),
            Self::Git {
                url,
                commit: Some(c),
            } => write!(f, "git:{url}#{c}"),
            Self::Registry(spec) => write!(f, "registry:{spec}"),
        }
    }
}

impl From<PluginSource> for String {
    fn from(source: PluginSource) -> Self {
        source.to_string()
    }
}

impl TryFrom<String> for PluginSource {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s).ok_or_else(|| format!("invalid plugin source: {s}"))
    }
}

impl PluginSource {
    /// Parse a source string like `local:./path`, `openclaw:@scope/pkg@1.0`,
    /// `git:https://...#commit`, or `registry:name@version`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let (prefix, value) = s.split_once(':')?;
        match prefix {
            "local" => Some(Self::Local(value.to_string())),
            "openclaw" => Some(Self::OpenClaw(value.to_string())),
            "git" => {
                // git:url or git:url#commit — note URL may contain ':'
                if let Some((url, commit)) = value.rsplit_once('#') {
                    Some(Self::Git {
                        url: url.to_string(),
                        commit: Some(commit.to_string()),
                    })
                } else {
                    Some(Self::Git {
                        url: value.to_string(),
                        commit: None,
                    })
                }
            },
            "registry" => Some(Self::Registry(value.to_string())),
            _ => None,
        }
    }
}

/// An integrity violation found during lockfile verification.
#[derive(Debug, Clone)]
pub enum IntegrityViolation {
    /// Plugin exists in lockfile but is missing from disk.
    Missing {
        /// The plugin that's missing.
        plugin_id: PluginId,
    },
    /// The WASM module hash doesn't match the lockfile.
    HashMismatch {
        /// The plugin with mismatched hash.
        plugin_id: PluginId,
        /// Hash recorded in the lockfile.
        expected: String,
        /// Hash computed from the file on disk.
        actual: String,
    },
    /// The manifest version doesn't match the lockfile.
    VersionMismatch {
        /// The plugin with mismatched version.
        plugin_id: PluginId,
        /// Version recorded in the lockfile.
        expected: String,
        /// Version found in the manifest on disk.
        actual: String,
    },
}

impl fmt::Display for IntegrityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { plugin_id } => {
                write!(f, "plugin {plugin_id} is in lockfile but missing from disk")
            },
            Self::HashMismatch {
                plugin_id,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "plugin {plugin_id}: WASM hash mismatch (expected {expected}, got {actual})"
                )
            },
            Self::VersionMismatch {
                plugin_id,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "plugin {plugin_id}: version mismatch (expected {expected}, got {actual})"
                )
            },
        }
    }
}

impl PluginLockfile {
    /// Create an empty lockfile.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }

    /// Load a lockfile from disk.
    ///
    /// Acquires a shared (read) lock on a `.lk` sibling file to coordinate
    /// with concurrent writers.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: &Path) -> PluginResult<Self> {
        let _lock_guard = acquire_lock_file(path, LockMode::Shared)?;

        let content = std::fs::read_to_string(path).map_err(|e| PluginError::LockfileError {
            path: path.to_path_buf(),
            message: format!("failed to read lockfile: {e}"),
        })?;

        Self::parse_content(path, &content)
    }

    /// Load a lockfile from disk, returning an empty lockfile if the file
    /// doesn't exist.
    ///
    /// Uses an atomic read pattern to avoid TOCTOU races: attempts to read
    /// the file and handles `NotFound` instead of checking `exists()` first.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load_or_default(path: &Path) -> PluginResult<Self> {
        let _lock_guard = acquire_lock_file(path, LockMode::Shared)?;

        match std::fs::read_to_string(path) {
            Ok(content) => Self::parse_content(path, &content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(PluginError::LockfileError {
                path: path.to_path_buf(),
                message: format!("failed to read lockfile: {e}"),
            }),
        }
    }

    /// Parse lockfile content from a string (shared between load methods).
    fn parse_content(path: &Path, content: &str) -> PluginResult<Self> {
        let lockfile: Self = toml::from_str(content).map_err(|e| PluginError::LockfileError {
            path: path.to_path_buf(),
            message: format!("failed to parse lockfile: {e}"),
        })?;

        if lockfile.schema_version != SCHEMA_VERSION {
            warn!(
                path = %path.display(),
                found = lockfile.schema_version,
                expected = SCHEMA_VERSION,
                "Lockfile schema version mismatch — attempting best-effort load"
            );
        }

        debug!(
            path = %path.display(),
            entries = lockfile.entries.len(),
            "Loaded plugin lockfile"
        );

        Ok(lockfile)
    }

    /// Atomically load, mutate, and save the lockfile under a single
    /// exclusive lock.
    ///
    /// Prevents TOCTOU races between concurrent `plugin install` / `plugin
    /// remove` operations that would otherwise load → drop lock → re-acquire
    /// → save, allowing another process to interleave and lose entries.
    ///
    /// If the lockfile doesn't exist, the closure receives an empty lockfile.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, parsed, or written, or
    /// if the closure returns an error.
    pub fn update<F>(path: &Path, f: F) -> PluginResult<()>
    where
        F: FnOnce(&mut Self) -> PluginResult<()>,
    {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PluginError::LockfileError {
                path: path.to_path_buf(),
                message: format!("failed to create parent directory: {e}"),
            })?;
        }

        // Hold the exclusive lock across both load and save.
        let _lock_guard = acquire_lock_file(path, LockMode::Exclusive)?;

        let mut lockfile = match std::fs::read_to_string(path) {
            Ok(content) => Self::parse_content(path, &content)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::new(),
            Err(e) => {
                return Err(PluginError::LockfileError {
                    path: path.to_path_buf(),
                    message: format!("failed to read lockfile: {e}"),
                });
            },
        };

        f(&mut lockfile)?;

        lockfile.save_inner(path)?;
        Ok(())
    }

    /// Save the lockfile to disk atomically.
    ///
    /// Writes to a temporary file in the same directory (same filesystem),
    /// then atomically renames it into place. Acquires an exclusive lock
    /// on a `.lk` sibling file to coordinate with concurrent readers/writers.
    ///
    /// Creates parent directories if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> PluginResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PluginError::LockfileError {
                path: path.to_path_buf(),
                message: format!("failed to create parent directory: {e}"),
            })?;
        }

        let _lock_guard = acquire_lock_file(path, LockMode::Exclusive)?;
        self.save_inner(path)
    }

    /// Inner save logic (caller must already hold the exclusive lock).
    fn save_inner(&self, path: &Path) -> PluginResult<()> {
        let header = "# Auto-generated by astrid. Do not edit manually.\n\n";
        let body = toml::to_string_pretty(self).map_err(|e| PluginError::LockfileError {
            path: path.to_path_buf(),
            message: format!("failed to serialize lockfile: {e}"),
        })?;

        let content = format!("{header}{body}");

        // Write to a temp file in the same directory, then atomically rename.
        let parent = path.parent().unwrap_or(Path::new("."));
        let mut tmp =
            tempfile::NamedTempFile::new_in(parent).map_err(|e| PluginError::LockfileError {
                path: path.to_path_buf(),
                message: format!("failed to create temp file for atomic write: {e}"),
            })?;

        tmp.write_all(content.as_bytes())
            .map_err(|e| PluginError::LockfileError {
                path: path.to_path_buf(),
                message: format!("failed to write temp lockfile: {e}"),
            })?;

        // Sync data to disk before renaming. Without this, a power loss
        // between persist() and the OS flushing dirty pages could leave
        // the lockfile empty or truncated. Worth the fsync cost for a file
        // that guards supply-chain integrity hashes.
        tmp.as_file()
            .sync_all()
            .map_err(|e| PluginError::LockfileError {
                path: path.to_path_buf(),
                message: format!("failed to sync temp lockfile to disk: {e}"),
            })?;

        tmp.persist(path).map_err(|e| PluginError::LockfileError {
            path: path.to_path_buf(),
            message: format!("failed to atomically replace lockfile: {e}"),
        })?;

        debug!(path = %path.display(), entries = self.entries.len(), "Saved plugin lockfile");
        Ok(())
    }

    /// Add or update a locked plugin entry.
    ///
    /// If a plugin with the same ID already exists, it is replaced.
    pub fn add(&mut self, entry: LockedPlugin) {
        self.remove(&entry.id);
        self.entries.push(entry);
    }

    /// Remove a plugin entry by ID.
    ///
    /// Returns `true` if an entry was removed.
    pub fn remove(&mut self, id: &PluginId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != *id);
        self.entries.len() < before
    }

    /// Look up a locked plugin entry by ID.
    #[must_use]
    pub fn get(&self, id: &PluginId) -> Option<&LockedPlugin> {
        self.entries.iter().find(|e| e.id == *id)
    }

    /// Get all locked plugin entries.
    #[must_use]
    pub fn entries(&self) -> &[LockedPlugin] {
        &self.entries
    }

    /// Check whether the lockfile has any entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of locked plugin entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Verify the integrity of installed plugins against the lockfile.
    ///
    /// For each entry, checks:
    /// 1. The plugin directory and manifest exist on disk.
    /// 2. The manifest version matches the lockfile version.
    /// 3. The WASM module blake3 hash matches the lockfile hash.
    ///
    /// Returns a list of violations (empty if everything is consistent).
    pub fn verify_integrity(&self, plugin_dir: &Path) -> Vec<IntegrityViolation> {
        let mut violations = Vec::new();

        for entry in &self.entries {
            let plugin_path = plugin_dir.join(entry.id.as_str());
            let manifest_path = plugin_path.join(MANIFEST_FILE_NAME);

            // 1. Check plugin directory exists
            if !manifest_path.exists() {
                violations.push(IntegrityViolation::Missing {
                    plugin_id: entry.id.clone(),
                });
                continue;
            }

            // 2. Load and check manifest version
            match crate::discovery::load_manifest(&manifest_path) {
                Ok(manifest) => {
                    if manifest.version != entry.version {
                        violations.push(IntegrityViolation::VersionMismatch {
                            plugin_id: entry.id.clone(),
                            expected: entry.version.clone(),
                            actual: manifest.version.clone(),
                        });
                    }

                    // 3. Check WASM hash if entry point is Wasm
                    if let crate::manifest::PluginEntryPoint::Wasm { path, .. } =
                        &manifest.entry_point
                    {
                        let wasm_path = if path.is_absolute() {
                            path.clone()
                        } else {
                            plugin_path.join(path)
                        };

                        match std::fs::read(&wasm_path) {
                            Ok(wasm_bytes) => {
                                let actual_hash =
                                    format!("blake3:{}", blake3::hash(&wasm_bytes).to_hex());
                                if actual_hash != entry.wasm_hash {
                                    violations.push(IntegrityViolation::HashMismatch {
                                        plugin_id: entry.id.clone(),
                                        expected: entry.wasm_hash.clone(),
                                        actual: actual_hash,
                                    });
                                }
                            },
                            Err(e) => {
                                warn!(
                                    plugin = %entry.id,
                                    path = %wasm_path.display(),
                                    error = %e,
                                    "Failed to read WASM file for integrity check"
                                );
                                violations.push(IntegrityViolation::Missing {
                                    plugin_id: entry.id.clone(),
                                });
                            },
                        }
                    }
                },
                Err(e) => {
                    warn!(
                        plugin = %entry.id,
                        error = %e,
                        "Failed to load manifest for integrity check"
                    );
                    violations.push(IntegrityViolation::Missing {
                        plugin_id: entry.id.clone(),
                    });
                },
            }
        }

        violations
    }
}

impl Default for PluginLockfile {
    fn default() -> Self {
        Self::new()
    }
}

impl LockedPlugin {
    /// Create a new locked plugin entry with the current timestamp.
    #[must_use]
    pub fn new(id: PluginId, version: String, source: PluginSource, wasm_hash: String) -> Self {
        Self {
            id,
            version,
            source,
            wasm_hash,
            installed_at: Utc::now(),
        }
    }

    /// Compute the blake3 hash of a WASM file and return it in lockfile
    /// format (`blake3:<hex>`).
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file cannot be read.
    pub fn compute_wasm_hash(wasm_path: &Path) -> PluginResult<String> {
        let bytes = std::fs::read(wasm_path)?;
        Ok(format!("blake3:{}", blake3::hash(&bytes).to_hex()))
    }

    /// Create a locked entry from a manifest on disk.
    ///
    /// Reads the WASM file (if the entry point is WASM) and computes its
    /// blake3 hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the WASM file can't be read.
    pub fn from_manifest(
        manifest: &PluginManifest,
        plugin_dir: &Path,
        source: PluginSource,
    ) -> PluginResult<Self> {
        let wasm_hash = match &manifest.entry_point {
            crate::manifest::PluginEntryPoint::Wasm { path, .. } => {
                let wasm_path = if path.is_absolute() {
                    path.clone()
                } else {
                    plugin_dir.join(path)
                };
                Self::compute_wasm_hash(&wasm_path)?
            },
            crate::manifest::PluginEntryPoint::Mcp { .. } => {
                // MCP plugins don't have a WASM file; use an empty sentinel.
                "none".to_string()
            },
        };

        Ok(Self::new(
            manifest.id.clone(),
            manifest.version.clone(),
            source,
            wasm_hash,
        ))
    }
}

/// Whether to acquire a shared (read) or exclusive (write) lock.
#[derive(Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

/// Acquire an advisory file lock on a `.lk` sibling of the given path.
///
/// Returns `Some(file)` holding the lock (dropped = released), or `None`
/// if the lock file doesn't exist and we're in shared (read) mode —
/// there's nothing to coordinate with, so no lock is needed.
///
/// In exclusive (write) mode, the lock file and parent directories are
/// created if they don't exist.
fn acquire_lock_file(lockfile_path: &Path, mode: LockMode) -> PluginResult<Option<std::fs::File>> {
    let lock_path = lockfile_path.with_extension("lk");

    match mode {
        LockMode::Shared => {
            // Read path: don't create any artifacts. If the lock file
            // doesn't exist, there's no concurrent writer to coordinate
            // with, so skip locking entirely.
            match std::fs::OpenOptions::new().read(true).open(&lock_path) {
                Ok(lock_file) => {
                    lock_file
                        .lock_shared()
                        .map_err(|e| PluginError::LockfileError {
                            path: lockfile_path.to_path_buf(),
                            message: format!("failed to acquire shared file lock: {e}"),
                        })?;
                    Ok(Some(lock_file))
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(PluginError::LockfileError {
                    path: lockfile_path.to_path_buf(),
                    message: format!("failed to open lock file: {e}"),
                }),
            }
        },
        LockMode::Exclusive => {
            // Write path: create directories and lock file as needed.
            if let Some(parent) = lock_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| PluginError::LockfileError {
                    path: lockfile_path.to_path_buf(),
                    message: format!("failed to create lock file directory: {e}"),
                })?;
            }

            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .read(true)
                .open(&lock_path)
                .map_err(|e| PluginError::LockfileError {
                    path: lockfile_path.to_path_buf(),
                    message: format!("failed to open lock file: {e}"),
                })?;

            lock_file
                .lock_exclusive()
                .map_err(|e| PluginError::LockfileError {
                    path: lockfile_path.to_path_buf(),
                    message: format!("failed to acquire exclusive file lock: {e}"),
                })?;

            Ok(Some(lock_file))
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn source_parse_local() {
        let s = PluginSource::parse("local:./plugins/my-plugin").unwrap();
        assert_eq!(s, PluginSource::Local("./plugins/my-plugin".to_string()));
        assert_eq!(s.to_string(), "local:./plugins/my-plugin");
    }

    #[test]
    fn source_parse_openclaw() {
        let s = PluginSource::parse("openclaw:@unicitylabs/hello-tool@1.0.0").unwrap();
        assert_eq!(
            s,
            PluginSource::OpenClaw("@unicitylabs/hello-tool@1.0.0".to_string())
        );
        assert_eq!(s.to_string(), "openclaw:@unicitylabs/hello-tool@1.0.0");
    }

    #[test]
    fn source_parse_git_with_commit() {
        let s = PluginSource::parse("git:https://github.com/user/repo#abc123").unwrap();
        assert_eq!(
            s,
            PluginSource::Git {
                url: "https://github.com/user/repo".to_string(),
                commit: Some("abc123".to_string()),
            }
        );
        assert_eq!(s.to_string(), "git:https://github.com/user/repo#abc123");
    }

    #[test]
    fn source_parse_git_without_commit() {
        let s = PluginSource::parse("git:https://github.com/user/repo").unwrap();
        assert_eq!(
            s,
            PluginSource::Git {
                url: "https://github.com/user/repo".to_string(),
                commit: None,
            }
        );
        assert_eq!(s.to_string(), "git:https://github.com/user/repo");
    }

    #[test]
    fn source_parse_registry() {
        let s = PluginSource::parse("registry:my-plugin@1.0.0").unwrap();
        assert_eq!(s, PluginSource::Registry("my-plugin@1.0.0".to_string()));
        assert_eq!(s.to_string(), "registry:my-plugin@1.0.0");
    }

    #[test]
    fn source_parse_invalid() {
        assert!(PluginSource::parse("ftp:something").is_none());
        assert!(PluginSource::parse("no-colon").is_none());
    }

    #[test]
    fn source_serde_round_trip() {
        let sources = vec![
            PluginSource::Local("./path".into()),
            PluginSource::OpenClaw("@scope/pkg@1.0".into()),
            PluginSource::Git {
                url: "https://github.com/user/repo".into(),
                commit: Some("abc".into()),
            },
            PluginSource::Registry("name@1.0".into()),
        ];

        for source in sources {
            let json = serde_json::to_string(&source).unwrap();
            let parsed: PluginSource = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, source);
        }
    }

    #[test]
    fn empty_lockfile() {
        let lf = PluginLockfile::new();
        assert!(lf.is_empty());
        assert_eq!(lf.len(), 0);
    }

    #[test]
    fn add_and_get() {
        let mut lf = PluginLockfile::new();
        let id = PluginId::from_static("test-plugin");
        let entry = LockedPlugin::new(
            id.clone(),
            "1.0.0".into(),
            PluginSource::Local("./plugins/test".into()),
            "blake3:abc123".into(),
        );
        lf.add(entry);

        assert_eq!(lf.len(), 1);
        let found = lf.get(&id).unwrap();
        assert_eq!(found.version, "1.0.0");
        assert_eq!(found.wasm_hash, "blake3:abc123");
    }

    #[test]
    fn add_replaces_existing() {
        let mut lf = PluginLockfile::new();
        let id = PluginId::from_static("test-plugin");

        lf.add(LockedPlugin::new(
            id.clone(),
            "1.0.0".into(),
            PluginSource::Local("./old".into()),
            "blake3:old".into(),
        ));
        lf.add(LockedPlugin::new(
            id.clone(),
            "2.0.0".into(),
            PluginSource::Local("./new".into()),
            "blake3:new".into(),
        ));

        assert_eq!(lf.len(), 1);
        assert_eq!(lf.get(&id).unwrap().version, "2.0.0");
    }

    #[test]
    fn remove_entry() {
        let mut lf = PluginLockfile::new();
        let id = PluginId::from_static("test-plugin");
        lf.add(LockedPlugin::new(
            id.clone(),
            "1.0.0".into(),
            PluginSource::Local("./path".into()),
            "blake3:hash".into(),
        ));

        assert!(lf.remove(&id));
        assert!(lf.is_empty());
        assert!(!lf.remove(&id)); // already removed
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let lockfile_path = dir.path().join(LOCKFILE_NAME);

        let mut lf = PluginLockfile::new();
        lf.add(LockedPlugin::new(
            PluginId::from_static("hello-tool"),
            "1.0.0".into(),
            PluginSource::OpenClaw("@unicitylabs/hello-tool@1.0.0".into()),
            "blake3:abc123def456".into(),
        ));
        lf.add(LockedPlugin::new(
            PluginId::from_static("github-tools"),
            "0.3.1".into(),
            PluginSource::Local("./plugins/github-tools".into()),
            "blake3:def456abc789".into(),
        ));

        lf.save(&lockfile_path).unwrap();

        // Verify the file starts with the comment header
        let content = std::fs::read_to_string(&lockfile_path).unwrap();
        assert!(content.starts_with("# Auto-generated by astrid."));
        assert!(content.contains("schema_version = 1"));

        // Load and verify
        let loaded = PluginLockfile::load(&lockfile_path).unwrap();
        assert_eq!(loaded.len(), 2);

        let hello = loaded.get(&PluginId::from_static("hello-tool")).unwrap();
        assert_eq!(hello.version, "1.0.0");
        assert_eq!(hello.wasm_hash, "blake3:abc123def456");
        assert_eq!(
            hello.source,
            PluginSource::OpenClaw("@unicitylabs/hello-tool@1.0.0".into())
        );

        let github = loaded.get(&PluginId::from_static("github-tools")).unwrap();
        assert_eq!(github.version, "0.3.1");
    }

    #[test]
    fn load_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("does_not_exist.lock");
        let result = PluginLockfile::load(&nonexistent);
        assert!(result.is_err());
    }

    #[test]
    fn load_or_default_nonexistent() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("does_not_exist.lock");
        let lf = PluginLockfile::load_or_default(&nonexistent).unwrap();
        assert!(lf.is_empty());
    }

    #[test]
    fn verify_integrity_all_good() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path();

        // Create a plugin on disk
        let plugin_path = plugin_dir.join("my-plugin");
        std::fs::create_dir(&plugin_path).unwrap();

        let wasm_data = b"fake wasm module bytes";
        let wasm_hash = format!("blake3:{}", blake3::hash(wasm_data).to_hex());
        std::fs::write(plugin_path.join("plugin.wasm"), wasm_data).unwrap();
        std::fs::write(
            plugin_path.join("plugin.toml"),
            r#"
id = "my-plugin"
name = "My Plugin"
version = "1.0.0"

[entry_point]
type = "wasm"
path = "plugin.wasm"
"#,
        )
        .unwrap();

        let mut lf = PluginLockfile::new();
        lf.add(LockedPlugin::new(
            PluginId::from_static("my-plugin"),
            "1.0.0".into(),
            PluginSource::Local("./plugins/my-plugin".into()),
            wasm_hash,
        ));

        let violations = lf.verify_integrity(plugin_dir);
        assert!(
            violations.is_empty(),
            "expected no violations, got: {violations:?}"
        );
    }

    #[test]
    fn verify_integrity_missing_plugin() {
        let dir = TempDir::new().unwrap();
        let mut lf = PluginLockfile::new();
        lf.add(LockedPlugin::new(
            PluginId::from_static("ghost-plugin"),
            "1.0.0".into(),
            PluginSource::Local("./nowhere".into()),
            "blake3:doesntmatter".into(),
        ));

        let violations = lf.verify_integrity(dir.path());
        assert_eq!(violations.len(), 1);
        assert!(matches!(
            &violations[0],
            IntegrityViolation::Missing { plugin_id } if plugin_id.as_str() == "ghost-plugin"
        ));
    }

    #[test]
    fn verify_integrity_hash_mismatch() {
        let dir = TempDir::new().unwrap();
        let plugin_path = dir.path().join("tampered-plugin");
        std::fs::create_dir(&plugin_path).unwrap();

        std::fs::write(plugin_path.join("plugin.wasm"), b"original bytes").unwrap();
        std::fs::write(
            plugin_path.join("plugin.toml"),
            r#"
id = "tampered-plugin"
name = "Tampered"
version = "1.0.0"

[entry_point]
type = "wasm"
path = "plugin.wasm"
"#,
        )
        .unwrap();

        let mut lf = PluginLockfile::new();
        lf.add(LockedPlugin::new(
            PluginId::from_static("tampered-plugin"),
            "1.0.0".into(),
            PluginSource::Local("./plugins/tampered".into()),
            "blake3:0000000000000000000000000000000000000000000000000000000000000000".into(),
        ));

        let violations = lf.verify_integrity(dir.path());
        assert_eq!(violations.len(), 1);
        assert!(matches!(
            &violations[0],
            IntegrityViolation::HashMismatch { plugin_id, .. } if plugin_id.as_str() == "tampered-plugin"
        ));
    }

    #[test]
    fn verify_integrity_version_mismatch() {
        let dir = TempDir::new().unwrap();
        let plugin_path = dir.path().join("outdated-plugin");
        std::fs::create_dir(&plugin_path).unwrap();

        let wasm_data = b"some wasm";
        let wasm_hash = format!("blake3:{}", blake3::hash(wasm_data).to_hex());
        std::fs::write(plugin_path.join("plugin.wasm"), wasm_data).unwrap();
        std::fs::write(
            plugin_path.join("plugin.toml"),
            r#"
id = "outdated-plugin"
name = "Outdated"
version = "2.0.0"

[entry_point]
type = "wasm"
path = "plugin.wasm"
"#,
        )
        .unwrap();

        let mut lf = PluginLockfile::new();
        lf.add(LockedPlugin::new(
            PluginId::from_static("outdated-plugin"),
            "1.0.0".into(),
            PluginSource::Local("./plugins/outdated".into()),
            wasm_hash,
        ));

        let violations = lf.verify_integrity(dir.path());
        assert_eq!(violations.len(), 1);
        assert!(matches!(
            &violations[0],
            IntegrityViolation::VersionMismatch { plugin_id, expected, actual }
            if plugin_id.as_str() == "outdated-plugin" && expected == "1.0.0" && actual == "2.0.0"
        ));
    }

    #[test]
    fn compute_wasm_hash_format() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("test.wasm");
        std::fs::write(&wasm_path, b"test data").unwrap();

        let hash = LockedPlugin::compute_wasm_hash(&wasm_path).unwrap();
        assert!(hash.starts_with("blake3:"));
        // blake3 hex is 64 chars
        assert_eq!(hash.len(), 7 + 64); // "blake3:" + 64 hex chars
    }

    #[test]
    fn locked_plugin_from_manifest() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path();
        let wasm_data = b"wasm module content";
        std::fs::write(plugin_dir.join("plugin.wasm"), wasm_data).unwrap();

        let manifest = PluginManifest {
            id: PluginId::from_static("from-manifest"),
            name: "From Manifest".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            entry_point: crate::manifest::PluginEntryPoint::Wasm {
                path: PathBuf::from("plugin.wasm"),
                hash: None,
            },
            capabilities: vec![],
            config: std::collections::HashMap::new(),
        };

        let entry = LockedPlugin::from_manifest(
            &manifest,
            plugin_dir,
            PluginSource::Local("./plugins/from-manifest".into()),
        )
        .unwrap();

        assert_eq!(entry.id.as_str(), "from-manifest");
        assert_eq!(entry.version, "1.0.0");
        assert!(entry.wasm_hash.starts_with("blake3:"));
        let expected_hash = format!("blake3:{}", blake3::hash(wasm_data).to_hex());
        assert_eq!(entry.wasm_hash, expected_hash);
    }

    #[test]
    fn toml_format_matches_spec() {
        let mut lf = PluginLockfile::new();
        lf.add(LockedPlugin {
            id: PluginId::from_static("hello-tool"),
            version: "1.0.0".into(),
            source: PluginSource::OpenClaw("@unicitylabs/hello-tool@1.0.0".into()),
            wasm_hash: "blake3:abc123".into(),
            installed_at: DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        });

        let toml_str = toml::to_string_pretty(&lf).unwrap();
        // Should contain the expected TOML structure
        assert!(toml_str.contains("schema_version = 1"));
        assert!(toml_str.contains("[[plugin]]"));
        assert!(toml_str.contains("id = \"hello-tool\""));
        assert!(toml_str.contains("version = \"1.0.0\""));
        assert!(toml_str.contains("source = \"openclaw:@unicitylabs/hello-tool@1.0.0\""));
        assert!(toml_str.contains("wasm_hash = \"blake3:abc123\""));
    }
}
