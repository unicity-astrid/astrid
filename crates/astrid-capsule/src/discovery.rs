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
    let local_plugins_dir = PathBuf::from(".astrid/plugins");
    if local_plugins_dir.exists() {
        info!(path = %local_plugins_dir.display(), "Discovering capsules from local directory");
        match load_manifests_from_dir(&local_plugins_dir) {
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

    Ok(manifest)
}

/// Capsules directory in a workspace.
#[must_use]
pub fn workspace_plugins_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".astrid").join("plugins")
}
