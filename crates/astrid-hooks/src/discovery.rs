//! Hook discovery - find hooks from standard locations.

use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::hook::Hook;

/// Errors that can occur during hook discovery.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// Failed to read directory.
    #[error("failed to read directory {path}: {message}")]
    DirectoryReadFailed {
        /// The path that failed.
        path: PathBuf,
        /// Error message.
        message: String,
    },

    /// Failed to read hook file.
    #[error("failed to read hook file {path}: {message}")]
    FileReadFailed {
        /// The path that failed.
        path: PathBuf,
        /// Error message.
        message: String,
    },

    /// Failed to parse hook file.
    #[error("failed to parse hook file {path}: {message}")]
    ParseFailed {
        /// The path that failed.
        path: PathBuf,
        /// Error message.
        message: String,
    },
}

/// Result type for discovery operations.
pub type DiscoveryResult<T> = Result<T, DiscoveryError>;

/// Standard hook file names.
pub const HOOK_FILE_NAMES: &[&str] = &["HOOK.toml", "hook.toml", "hooks.toml"];

/// Discover hooks from standard locations.
///
/// This function looks for hooks in:
/// 1. `.astrid/hooks/` in the current directory (workspace-level)
/// 2. Any additional paths provided in `extra_paths`
///
/// Callers should pass the user-level hooks directory (e.g.
/// `AstridHome::hooks_dir()`) via `extra_paths` rather than relying
/// on hard-coded platform paths.
pub fn discover_hooks(extra_paths: Option<&[PathBuf]>) -> Vec<Hook> {
    let mut hooks = Vec::new();

    // Look in local .astrid/hooks directory
    let local_hooks_dir = PathBuf::from(".astrid/hooks");
    if local_hooks_dir.exists() {
        info!(path = %local_hooks_dir.display(), "Discovering hooks from local directory");
        match load_hooks_from_dir(&local_hooks_dir) {
            Ok(found) => hooks.extend(found),
            Err(e) => warn!(error = %e, "Failed to load hooks from local directory"),
        }
    }

    // Look in extra paths
    if let Some(paths) = extra_paths {
        for path in paths {
            if path.exists() {
                info!(path = %path.display(), "Discovering hooks from custom path");
                match load_hooks_from_dir(path) {
                    Ok(found) => hooks.extend(found),
                    Err(e) => warn!(error = %e, "Failed to load hooks from custom path"),
                }
            }
        }
    }

    info!(count = hooks.len(), "Discovered hooks");
    hooks
}

/// Load hooks from a directory.
///
/// This function looks for:
/// - Direct hook files (HOOK.toml, hook.toml)
/// - Subdirectories containing hook files
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn load_hooks_from_dir(dir: &Path) -> DiscoveryResult<Vec<Hook>> {
    let mut hooks = Vec::new();

    let entries = std::fs::read_dir(dir).map_err(|e| DiscoveryError::DirectoryReadFailed {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| DiscoveryError::DirectoryReadFailed {
            path: dir.to_path_buf(),
            message: e.to_string(),
        })?;

        let path = entry.path();

        if path.is_dir() {
            // Look for hook file in subdirectory
            for hook_file in HOOK_FILE_NAMES {
                let hook_path = path.join(hook_file);
                if hook_path.exists() {
                    match load_hook(&hook_path) {
                        Ok(hook) => {
                            debug!(path = %hook_path.display(), "Loaded hook");
                            hooks.push(hook);
                        },
                        Err(e) => {
                            warn!(
                                path = %hook_path.display(),
                                error = %e,
                                "Failed to load hook"
                            );
                        },
                    }
                    break; // Only load first matching file
                }
            }
        } else if path.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
            && HOOK_FILE_NAMES.contains(&name)
        {
            match load_hook(&path) {
                Ok(hook) => {
                    debug!(path = %path.display(), "Loaded hook");
                    hooks.push(hook);
                },
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to load hook");
                },
            }
        }
    }

    Ok(hooks)
}

/// Load a single hook from a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn load_hook(path: &Path) -> DiscoveryResult<Hook> {
    let content = std::fs::read_to_string(path).map_err(|e| DiscoveryError::FileReadFailed {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    let hook: Hook = toml::from_str(&content).map_err(|e| DiscoveryError::ParseFailed {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    Ok(hook)
}

/// Save a hook to a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn save_hook(hook: &Hook, path: &Path) -> DiscoveryResult<()> {
    let content = toml::to_string_pretty(hook).map_err(|e| DiscoveryError::ParseFailed {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    std::fs::write(path, content).map_err(|e| DiscoveryError::FileReadFailed {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Hooks directory in a workspace.
#[must_use]
pub fn workspace_hooks_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".astrid").join("hooks")
}

/// Ensure the hooks directory exists.
///
/// # Errors
///
/// Returns an error if the directory cannot be created.
pub fn ensure_hooks_dir(workspace_root: &Path) -> std::io::Result<PathBuf> {
    let dir = workspace_hooks_dir(workspace_root);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{HookEvent, HookHandler};
    use tempfile::TempDir;

    #[test]
    fn test_load_hook_from_toml() {
        let temp_dir = TempDir::new().unwrap();
        let hook_path = temp_dir.path().join("HOOK.toml");

        let hook = Hook::new(HookEvent::SessionStart)
            .with_name("test-hook")
            .with_handler(HookHandler::command("echo"));

        // Save the hook
        save_hook(&hook, &hook_path).unwrap();

        // Load it back
        let loaded = load_hook(&hook_path).unwrap();

        assert_eq!(loaded.name, Some("test-hook".to_string()));
        assert_eq!(loaded.event, HookEvent::SessionStart);
    }

    #[test]
    fn test_load_hooks_from_dir() {
        let temp_dir = TempDir::new().unwrap();

        // Create a subdirectory with a hook
        let subdir = temp_dir.path().join("my-hook");
        std::fs::create_dir(&subdir).unwrap();

        let hook = Hook::new(HookEvent::PreToolCall)
            .with_name("sub-hook")
            .with_handler(HookHandler::command("echo"));

        save_hook(&hook, &subdir.join("HOOK.toml")).unwrap();

        // Load hooks from the directory
        let hooks = load_hooks_from_dir(temp_dir.path()).unwrap();

        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].name, Some("sub-hook".to_string()));
    }

    #[test]
    fn test_discover_hooks_empty() {
        // Should not panic even with no hooks
        let hooks = discover_hooks(None);
        // May find system hooks, so just check it doesn't panic
        let _ = hooks;
    }
}
