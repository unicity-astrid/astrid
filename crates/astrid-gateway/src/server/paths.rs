//! Daemon state file paths.

use std::path::PathBuf;

/// Paths for daemon state files.
pub struct DaemonPaths {
    /// Directory for daemon files (e.g. `~/.astrid/`).
    pub base_dir: PathBuf,
}

impl DaemonPaths {
    /// Create paths for the default location using `AstridHome`.
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be resolved.
    pub fn default_dir() -> Result<Self, std::io::Error> {
        let home = astrid_core::dirs::AstridHome::resolve()?;
        Ok(Self {
            base_dir: home.root().to_path_buf(),
        })
    }

    /// PID file path.
    #[must_use]
    pub fn pid_file(&self) -> PathBuf {
        self.base_dir.join("daemon.pid")
    }

    /// Port file path (written on startup so CLI knows where to connect).
    #[must_use]
    pub fn port_file(&self) -> PathBuf {
        self.base_dir.join("daemon.port")
    }

    /// Daemon log file path (stderr is redirected here on auto-start).
    #[must_use]
    pub fn log_file(&self) -> PathBuf {
        self.base_dir.join("logs").join("daemon.log")
    }

    /// Mode file path (records whether daemon is ephemeral or persistent).
    #[must_use]
    pub fn mode_file(&self) -> PathBuf {
        self.base_dir.join("daemon.mode")
    }
}
