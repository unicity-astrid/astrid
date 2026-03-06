//! Capsule manifest discovery from standard locations.
//!
//! Scans well-known directories for `Capsule.toml` files, providing
//! the entry point for the Manifest-First architecture.

use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::CapsuleManifest;

/// Standard capsule manifest file name.
pub const MANIFEST_FILE_NAME: &str = "Capsule.toml";

/// Discover capsule manifests from standard locations.
///
/// Scans the following directories for `Capsule.toml` files:
/// 1. `.astrid/plugins/` (workspace-level, relative to CWD)
/// 2. Any additional paths provided in `extra_paths`
///
/// Each subdirectory containing a `Capsule.toml` is treated as a capsule.
/// Errors in individual manifests are logged as warnings but do not
/// prevent other manifests from loading.
///
/// Returns `(manifest, capsule_dir)` pairs where `capsule_dir` is the
/// directory containing the manifest.
pub fn discover_manifests(extra_paths: Option<&[PathBuf]>) -> Vec<(CapsuleManifest, PathBuf)> {
    let mut manifests = Vec::new();

    // Workspace-level capsules
    let local_capsules_dir = PathBuf::from(".astrid/capsules");
    if local_capsules_dir.exists() {
        info!(path = %local_capsules_dir.display(), "Discovering capsules from local directory");
        match load_manifests_from_dir(&local_capsules_dir) {
            Ok(found) => manifests.extend(found),
            Err(e) => warn!(error = %e, "Failed to load capsules from local directory"),
        }
    }

    // Extra paths (user-level, custom, etc.)
    if let Some(paths) = extra_paths {
        for path in paths {
            if path.exists() {
                info!(path = %path.display(), "Discovering capsules from custom path");
                match load_manifests_from_dir(path) {
                    Ok(found) => manifests.extend(found),
                    Err(e) => warn!(error = %e, "Failed to load capsules from custom path"),
                }
            }
        }
    }

    info!(count = manifests.len(), "Discovered capsule manifests");
    manifests
}

/// Load all capsule manifests from a directory.
///
/// Looks for subdirectories containing `Capsule.toml` files, as well as
/// `Capsule.toml` files directly in the directory.
pub fn load_manifests_from_dir(dir: &Path) -> CapsuleResult<Vec<(CapsuleManifest, PathBuf)>> {
    let mut manifests = Vec::new();

    let entries = std::fs::read_dir(dir).map_err(|e| CapsuleError::ManifestParseError {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| CapsuleError::ManifestParseError {
            path: dir.to_path_buf(),
            message: e.to_string(),
        })?;
        let path = entry.path();

        if path.is_dir() {
            // Look for Capsule.toml in subdirectory
            let manifest_path = path.join(MANIFEST_FILE_NAME);
            if manifest_path.exists() {
                match load_manifest(&manifest_path) {
                    Ok(manifest) => {
                        debug!(
                            path = %manifest_path.display(),
                            capsule_name = %manifest.package.name,
                            "Loaded capsule manifest"
                        );
                        manifests.push((manifest, path));
                    },
                    Err(e) => {
                        warn!(
                            path = %manifest_path.display(),
                            error = %e,
                            "Failed to load capsule manifest"
                        );
                    },
                }
            }
        } else if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == MANIFEST_FILE_NAME)
        {
            let plugin_dir = path.parent().unwrap_or(dir).to_path_buf();
            match load_manifest(&path) {
                Ok(manifest) => {
                    debug!(
                        path = %path.display(),
                        capsule_name = %manifest.package.name,
                        "Loaded capsule manifest"
                    );
                    manifests.push((manifest, plugin_dir));
                },
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to load capsule manifest");
                },
            }
        }
    }

    Ok(manifests)
}

/// Load a single capsule manifest from a TOML file.
pub fn load_manifest(path: &Path) -> CapsuleResult<CapsuleManifest> {
    let content = std::fs::read_to_string(path).map_err(|e| CapsuleError::ManifestParseError {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    let manifest: CapsuleManifest =
        toml::from_str(&content).map_err(|e| CapsuleError::ManifestParseError {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    // Validate ipc_publish patterns contain no empty segments.
    for pattern in &manifest.capabilities.ipc_publish {
        if !crate::dispatcher::has_valid_segments(pattern) {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!(
                    "ipc_publish pattern '{pattern}' contains empty segments \
                     (consecutive dots, leading/trailing dots, or is empty)"
                ),
            });
        }
    }

    // Validate interceptor event patterns contain no empty segments.
    for interceptor in &manifest.interceptors {
        if !crate::dispatcher::has_valid_segments(&interceptor.event) {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!(
                    "interceptor event pattern '{}' contains empty segments \
                     (consecutive dots, leading/trailing dots, or is empty)",
                    interceptor.event
                ),
            });
        }
    }

    Ok(manifest)
}

/// Capsules directory in a workspace.
#[must_use]
pub fn workspace_plugins_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".astrid").join("plugins")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a TOML string to a temp file and call `load_manifest`.
    fn load_from_toml(toml: &str) -> CapsuleResult<crate::manifest::CapsuleManifest> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Capsule.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(toml.as_bytes()).unwrap();
        load_manifest(&path)
    }

    const VALID_HEADER: &str = r#"
[package]
name = "test-capsule"
version = "0.1.0"
"#;

    #[test]
    fn load_manifest_accepts_valid_ipc_publish() {
        let toml = format!(
            "{VALID_HEADER}\n[capabilities]\nipc_publish = [\"registry.*\", \"llm.stream.anthropic\"]"
        );
        assert!(load_from_toml(&toml).is_ok());
    }

    #[test]
    fn load_manifest_rejects_empty_segment_in_ipc_publish() {
        for bad in &["a..b", ".a.b", "a.b.", "", ".", "a...b"] {
            let toml = format!("{VALID_HEADER}\n[capabilities]\nipc_publish = [\"{bad}\"]");
            let err = load_from_toml(&toml).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("empty segments"),
                "expected 'empty segments' error for pattern '{bad}', got: {msg}"
            );
        }
    }

    #[test]
    fn load_manifest_rejects_empty_segment_in_interceptor_event() {
        for bad in &["a..b", ".event", "event.", "", ".", "a...b"] {
            let toml =
                format!("{VALID_HEADER}\n[[interceptor]]\nevent = \"{bad}\"\naction = \"handle\"");
            let err = load_from_toml(&toml).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("empty segments"),
                "expected 'empty segments' error for event '{bad}', got: {msg}"
            );
        }
    }

    #[test]
    fn load_manifest_accepts_valid_interceptor_event() {
        let toml = format!(
            "{VALID_HEADER}\n[[interceptor]]\nevent = \"user.prompt\"\naction = \"handle\""
        );
        assert!(load_from_toml(&toml).is_ok());
    }
}
