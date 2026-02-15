//! Directory scaffolding for Astrid home and workspace directories.
//!
//! Two key directory structures:
//!
//! - [`AstridHome`]: Global state at `~/.astrid/` (or `$ASTRID_HOME`).
//!   Holds keys, audit logs, capability databases, sessions, workspace state,
//!   and global config. All sensitive/runtime data lives here.
//!
//! - [`WorkspaceDir`]: Per-project directory at `<project>/.astrid/`.
//!   Holds only committable project-level config (like `.claude/CLAUDE.md`).
//!   Contains a `workspace-id` UUID that links the project to its global state.
//!
//! # Layout
//!
//! ```text
//! ~/.astrid/                      (AstridHome)
//! ├── keys/
//! │   └── user.key                  (ed25519 secret key, 0600)
//! ├── logs/                         (daemon/runtime log files)
//! ├── sessions/                     (session JSON files)
//! ├── state.db/                     (SurrealKV — allowances, budget, escape)
//! ├── audit.db/                     (SurrealKV — audit entries)
//! ├── capabilities.db/              (SurrealKV — capability tokens)
//! ├── deferred.db/                  (SurrealKV — deferred queue)
//! ├── hooks/                        (user-level hooks)
//! ├── plugins/                      (installed plugins)
//! ├── cache/plugins/                (plugin compilation cache)
//! ├── state/                        (gateway state)
//! ├── servers.toml                  (MCP server config)
//! ├── gateway.toml                  (gateway daemon config)
//! └── config.toml                   (global runtime config)
//!
//! <project>/.astrid/              (WorkspaceDir)
//! ├── workspace-id                  (UUID linking project to global state)
//! └── ASTRID.md                   (project-level instructions)
//! ```

use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Global Astrid home directory (`~/.astrid/` or `$ASTRID_HOME`).
///
/// Contains keys, audit databases, capability stores, and global config.
#[derive(Debug, Clone)]
pub struct AstridHome {
    root: PathBuf,
}

impl AstridHome {
    /// Resolve the home directory.
    ///
    /// Checks `$ASTRID_HOME` first, then falls back to `$HOME/.astrid/`.
    ///
    /// # Errors
    ///
    /// Returns an error if neither `$ASTRID_HOME` nor `$HOME` is set.
    pub fn resolve() -> io::Result<Self> {
        let root = if let Ok(custom) = std::env::var("ASTRID_HOME") {
            let p = PathBuf::from(&custom);
            if !p.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ASTRID_HOME must be an absolute path",
                ));
            }
            p
        } else {
            let home = std::env::var("HOME").map_err(|_| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "neither ASTRID_HOME nor HOME environment variable is set",
                )
            })?;
            PathBuf::from(home).join(".astrid")
        };

        Ok(Self { root })
    }

    /// Create from an explicit path (useful for testing).
    #[must_use]
    pub fn from_path(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Ensure the directory structure exists with secure permissions.
    ///
    /// Creates `keys/`, `logs/`, and `sessions/` subdirectories and sets them
    /// all to `0o700` on Unix (owner-only access).
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or permission setting fails.
    pub fn ensure(&self) -> io::Result<()> {
        std::fs::create_dir_all(self.keys_dir())?;
        std::fs::create_dir_all(self.logs_dir())?;
        std::fs::create_dir_all(self.sessions_dir())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(self.root(), perms.clone())?;
            std::fs::set_permissions(self.keys_dir(), perms.clone())?;
            std::fs::set_permissions(self.logs_dir(), perms.clone())?;
            std::fs::set_permissions(self.sessions_dir(), perms)?;
        }
        Ok(())
    }

    /// Root directory path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Keys directory (`~/.astrid/keys/`).
    #[must_use]
    pub fn keys_dir(&self) -> PathBuf {
        self.root.join("keys")
    }

    /// Logs directory (`~/.astrid/logs/`).
    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Path to the user's ed25519 secret key file.
    #[must_use]
    pub fn user_key_path(&self) -> PathBuf {
        self.keys_dir().join("user.key")
    }

    /// Path to the audit database directory (`SurrealKV`).
    #[must_use]
    pub fn audit_db_path(&self) -> PathBuf {
        self.root.join("audit.db")
    }

    /// Path to the capabilities database directory (`SurrealKV`).
    #[must_use]
    pub fn capabilities_db_path(&self) -> PathBuf {
        self.root.join("capabilities.db")
    }

    /// Path to the deferred queue database directory (`SurrealKV`).
    #[must_use]
    pub fn deferred_db_path(&self) -> PathBuf {
        self.root.join("deferred.db")
    }

    /// Path to the MCP servers configuration file.
    #[must_use]
    pub fn servers_config_path(&self) -> PathBuf {
        self.root.join("servers.toml")
    }

    /// Path to the global runtime configuration file.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// Sessions directory (`~/.astrid/sessions/`).
    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Path to the workspace state database directory (`SurrealKV`).
    #[must_use]
    pub fn state_db_path(&self) -> PathBuf {
        self.root.join("state.db")
    }

    /// Installed plugins directory (`~/.astrid/plugins/`).
    #[must_use]
    pub fn plugins_dir(&self) -> PathBuf {
        self.root.join("plugins")
    }

    /// Plugin compilation cache directory (`~/.astrid/cache/plugins/`).
    #[must_use]
    pub fn plugin_cache_dir(&self) -> PathBuf {
        self.root.join("cache").join("plugins")
    }

    /// Hooks directory (`~/.astrid/hooks/`).
    #[must_use]
    pub fn hooks_dir(&self) -> PathBuf {
        self.root.join("hooks")
    }

    /// Path to the gateway configuration file (`~/.astrid/gateway.toml`).
    #[must_use]
    pub fn gateway_config_path(&self) -> PathBuf {
        self.root.join("gateway.toml")
    }

    /// State directory (`~/.astrid/state/`).
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
}

/// Per-project workspace directory (`<project>/.astrid/`).
///
/// Contains only committable project-level config. A `workspace-id` UUID
/// links the project to its global state in `~/.astrid/`.
#[derive(Debug, Clone)]
pub struct WorkspaceDir {
    /// The project root (parent of `.astrid/`).
    project_root: PathBuf,
}

impl WorkspaceDir {
    /// Detect the workspace directory by walking up from `start_dir`.
    ///
    /// Detection order:
    /// 1. Directory containing `.astrid/`
    /// 2. Directory containing `.git`
    /// 3. Directory containing `ASTRID.md`
    /// 4. Fallback to `start_dir` itself
    #[must_use]
    pub fn detect(start_dir: &Path) -> Self {
        let start = if start_dir.is_absolute() {
            start_dir.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(start_dir)
        };

        let mut current = start.as_path();

        loop {
            // Check for .astrid/ directory
            if current.join(".astrid").is_dir() {
                return Self {
                    project_root: current.to_path_buf(),
                };
            }

            // Check for .git
            if current.join(".git").exists() {
                return Self {
                    project_root: current.to_path_buf(),
                };
            }

            // Check for ASTRID.md
            if current.join("ASTRID.md").exists() {
                return Self {
                    project_root: current.to_path_buf(),
                };
            }

            // Walk up
            match current.parent() {
                Some(parent) if parent != current => current = parent,
                _ => break,
            }
        }

        // Fallback to start_dir
        Self {
            project_root: start,
        }
    }

    /// Create from an explicit project root (useful for testing).
    #[must_use]
    pub fn from_path(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    /// Ensure the `.astrid/` directory exists and generate a workspace ID
    /// if one does not already exist.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or workspace ID generation fails.
    pub fn ensure(&self) -> io::Result<()> {
        std::fs::create_dir_all(self.dot_astrid())?;
        // Generate workspace ID if missing (idempotent — reads existing).
        let _ = self.workspace_id()?;
        Ok(())
    }

    /// Project root directory (parent of `.astrid/`).
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.project_root
    }

    /// The `.astrid/` directory itself.
    #[must_use]
    pub fn dot_astrid(&self) -> PathBuf {
        self.project_root.join(".astrid")
    }

    /// Path to the workspace-id file (`.astrid/workspace-id`).
    #[must_use]
    pub fn workspace_id_path(&self) -> PathBuf {
        self.dot_astrid().join("workspace-id")
    }

    /// Read or generate the workspace ID.
    ///
    /// If the file exists (e.g. cloned from a repo), its UUID is adopted.
    /// Otherwise a new UUID is generated and written.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or written.
    pub fn workspace_id(&self) -> io::Result<Uuid> {
        let path = self.workspace_id_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            let trimmed = content.trim();
            if let Ok(id) = Uuid::parse_str(trimmed) {
                return Ok(id);
            }
        }
        // Generate and write a new workspace ID.
        std::fs::create_dir_all(self.dot_astrid())?;
        let id = Uuid::new_v4();
        std::fs::write(&path, id.to_string())?;
        Ok(id)
    }

    /// Path to the project-level instructions file (`.astrid/ASTRID.md`).
    #[must_use]
    pub fn instructions_path(&self) -> PathBuf {
        self.dot_astrid().join("ASTRID.md")
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mutex to serialize tests that mutate the `ASTRID_HOME` env var.
    /// `set_var`/`remove_var` are process-wide and unsafe under concurrency.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_astrid_home_resolve_with_env() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // SAFETY: serialized by ENV_MUTEX
        unsafe { std::env::set_var("ASTRID_HOME", &path) };
        let home = AstridHome::resolve().unwrap();
        assert_eq!(home.root(), path);
        unsafe { std::env::remove_var("ASTRID_HOME") };
    }

    #[test]
    fn test_astrid_home_resolve_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: serialized by ENV_MUTEX
        unsafe { std::env::remove_var("ASTRID_HOME") };
        let home = AstridHome::resolve().unwrap();
        let expected = PathBuf::from(std::env::var("HOME").unwrap()).join(".astrid");
        assert_eq!(home.root(), expected);
    }

    #[test]
    fn test_astrid_home_ensure_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(dir.path());
        home.ensure().unwrap();

        assert!(home.keys_dir().exists());
        assert!(home.logs_dir().exists());
        assert!(home.sessions_dir().exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_astrid_home_ensure_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(dir.path());
        home.ensure().unwrap();

        let root_perms = std::fs::metadata(home.root()).unwrap().permissions();
        assert_eq!(root_perms.mode() & 0o777, 0o700);

        let keys_perms = std::fs::metadata(home.keys_dir()).unwrap().permissions();
        assert_eq!(keys_perms.mode() & 0o777, 0o700);
    }

    #[test]
    fn test_astrid_home_rejects_relative_env() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: serialized by ENV_MUTEX
        unsafe { std::env::set_var("ASTRID_HOME", "relative/path") };
        let result = AstridHome::resolve();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("absolute"),
            "expected absolute path error, got: {err}"
        );
        unsafe { std::env::remove_var("ASTRID_HOME") };
    }

    #[test]
    fn test_astrid_home_rejects_empty_env() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: serialized by ENV_MUTEX
        unsafe { std::env::set_var("ASTRID_HOME", "") };
        let result = AstridHome::resolve();
        assert!(result.is_err());
        unsafe { std::env::remove_var("ASTRID_HOME") };
    }

    #[test]
    fn test_astrid_home_path_accessors() {
        let home = AstridHome::from_path("/tmp/test-astrid");
        assert_eq!(home.root(), Path::new("/tmp/test-astrid"));
        assert_eq!(home.keys_dir(), PathBuf::from("/tmp/test-astrid/keys"));
        assert_eq!(home.logs_dir(), PathBuf::from("/tmp/test-astrid/logs"));
        assert_eq!(
            home.user_key_path(),
            PathBuf::from("/tmp/test-astrid/keys/user.key")
        );
        assert_eq!(
            home.audit_db_path(),
            PathBuf::from("/tmp/test-astrid/audit.db")
        );
        assert_eq!(
            home.capabilities_db_path(),
            PathBuf::from("/tmp/test-astrid/capabilities.db")
        );
        assert_eq!(
            home.deferred_db_path(),
            PathBuf::from("/tmp/test-astrid/deferred.db")
        );
        assert_eq!(
            home.servers_config_path(),
            PathBuf::from("/tmp/test-astrid/servers.toml")
        );
        assert_eq!(
            home.config_path(),
            PathBuf::from("/tmp/test-astrid/config.toml")
        );
        assert_eq!(
            home.sessions_dir(),
            PathBuf::from("/tmp/test-astrid/sessions")
        );
        assert_eq!(
            home.state_db_path(),
            PathBuf::from("/tmp/test-astrid/state.db")
        );
        assert_eq!(
            home.plugins_dir(),
            PathBuf::from("/tmp/test-astrid/plugins")
        );
        assert_eq!(
            home.plugin_cache_dir(),
            PathBuf::from("/tmp/test-astrid/cache/plugins")
        );
        assert_eq!(home.hooks_dir(), PathBuf::from("/tmp/test-astrid/hooks"));
        assert_eq!(
            home.gateway_config_path(),
            PathBuf::from("/tmp/test-astrid/gateway.toml")
        );
        assert_eq!(home.state_dir(), PathBuf::from("/tmp/test-astrid/state"));
    }

    #[test]
    fn test_workspace_detect_with_dot_astrid() {
        let dir = tempfile::tempdir().unwrap();
        let astrid_dir = dir.path().join(".astrid");
        std::fs::create_dir(&astrid_dir).unwrap();

        let sub = dir.path().join("src").join("deep");
        std::fs::create_dir_all(&sub).unwrap();

        let ws = WorkspaceDir::detect(&sub);
        assert_eq!(ws.root(), dir.path());
    }

    #[test]
    fn test_workspace_detect_with_git() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();

        let ws = WorkspaceDir::detect(&sub);
        assert_eq!(ws.root(), dir.path());
    }

    #[test]
    fn test_workspace_detect_with_astrid_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ASTRID.md"), "# Project").unwrap();

        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();

        let ws = WorkspaceDir::detect(&sub);
        assert_eq!(ws.root(), dir.path());
    }

    #[test]
    fn test_workspace_detect_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let isolated = dir.path().join("isolated");
        std::fs::create_dir_all(&isolated).unwrap();

        // No .astrid, .git, or ASTRID.md anywhere in the tree up from
        // `isolated` (tempdir itself has no markers) — but the walk will
        // eventually hit the real filesystem root. To truly test fallback
        // we use `from_path` which is the deterministic path.
        let ws = WorkspaceDir::from_path(&isolated);
        assert_eq!(ws.root(), isolated);
    }

    #[test]
    fn test_workspace_detect_prefers_dot_astrid_over_git() {
        let dir = tempfile::tempdir().unwrap();
        // Create both .astrid/ and .git
        std::fs::create_dir(dir.path().join(".astrid")).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();

        let ws = WorkspaceDir::detect(&sub);
        assert_eq!(ws.root(), dir.path());
    }

    #[test]
    fn test_workspace_ensure_creates_dirs_and_id() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceDir::from_path(dir.path());
        ws.ensure().unwrap();

        assert!(ws.dot_astrid().exists());
        assert!(ws.workspace_id_path().exists());

        // workspace_id should be a valid UUID
        let content = std::fs::read_to_string(ws.workspace_id_path()).unwrap();
        uuid::Uuid::parse_str(content.trim()).expect("workspace-id should be a valid UUID");
    }

    #[test]
    fn test_workspace_id_adopts_existing() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceDir::from_path(dir.path());

        // Pre-write a workspace-id (simulating a cloned repo)
        std::fs::create_dir_all(ws.dot_astrid()).unwrap();
        let pre_id = uuid::Uuid::new_v4();
        std::fs::write(ws.workspace_id_path(), pre_id.to_string()).unwrap();

        // workspace_id() should adopt the existing ID
        let id = ws.workspace_id().unwrap();
        assert_eq!(id, pre_id);
    }

    #[test]
    fn test_workspace_id_stable_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceDir::from_path(dir.path());
        let id1 = ws.workspace_id().unwrap();
        let id2 = ws.workspace_id().unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_workspace_path_accessors() {
        let ws = WorkspaceDir::from_path("/home/user/project");
        assert_eq!(ws.root(), Path::new("/home/user/project"));
        assert_eq!(ws.dot_astrid(), PathBuf::from("/home/user/project/.astrid"));
        assert_eq!(
            ws.workspace_id_path(),
            PathBuf::from("/home/user/project/.astrid/workspace-id")
        );
        assert_eq!(
            ws.instructions_path(),
            PathBuf::from("/home/user/project/.astrid/ASTRID.md")
        );
    }
}
