//! Plugin manifest discovery from standard locations.
//!
//! Scans well-known directories for `plugin.toml` files, similar to
//! the hook discovery pattern in `astralis-hooks`.

use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::error::{PluginError, PluginResult};
use crate::manifest::PluginManifest;

/// Standard plugin manifest file name.
pub const MANIFEST_FILE_NAME: &str = "plugin.toml";

/// Discover plugin manifests from standard locations.
///
/// Scans the following directories for `plugin.toml` files:
/// 1. `.astralis/plugins/` (workspace-level, relative to CWD)
/// 2. Any additional paths provided in `extra_paths`
///
/// Callers should pass the user-level plugins directory (e.g.
/// `AstralisHome::plugins_dir()`) via `extra_paths` rather than relying
/// on hard-coded platform paths.
///
/// Each subdirectory containing a `plugin.toml` is treated as a plugin.
/// Errors in individual manifests are logged as warnings but do not
/// prevent other manifests from loading.
///
/// Returns `(manifest, plugin_dir)` pairs where `plugin_dir` is the
/// directory containing the manifest. Callers should resolve relative
/// entry-point paths against `plugin_dir`.
pub fn discover_manifests(extra_paths: Option<&[PathBuf]>) -> Vec<(PluginManifest, PathBuf)> {
    let mut manifests = Vec::new();

    // Workspace-level plugins
    let local_plugins_dir = PathBuf::from(".astralis/plugins");
    if local_plugins_dir.exists() {
        info!(path = %local_plugins_dir.display(), "Discovering plugins from local directory");
        match load_manifests_from_dir(&local_plugins_dir) {
            Ok(found) => manifests.extend(found),
            Err(e) => warn!(error = %e, "Failed to load plugins from local directory"),
        }
    }

    // Extra paths (user-level, custom, etc.)
    if let Some(paths) = extra_paths {
        for path in paths {
            if path.exists() {
                info!(path = %path.display(), "Discovering plugins from custom path");
                match load_manifests_from_dir(path) {
                    Ok(found) => manifests.extend(found),
                    Err(e) => warn!(error = %e, "Failed to load plugins from custom path"),
                }
            }
        }
    }

    info!(count = manifests.len(), "Discovered plugin manifests");
    manifests
}

/// Load all plugin manifests from a directory.
///
/// Looks for subdirectories containing `plugin.toml` files, as well as
/// `plugin.toml` files directly in the directory.
///
/// Returns `(manifest, plugin_dir)` pairs where `plugin_dir` is the
/// directory containing each manifest file.
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn load_manifests_from_dir(dir: &Path) -> PluginResult<Vec<(PluginManifest, PathBuf)>> {
    let mut manifests = Vec::new();

    let entries = std::fs::read_dir(dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Look for plugin.toml in subdirectory
            let manifest_path = path.join(MANIFEST_FILE_NAME);
            if manifest_path.exists() {
                match load_manifest(&manifest_path) {
                    Ok(manifest) => {
                        debug!(
                            path = %manifest_path.display(),
                            plugin_id = %manifest.id,
                            "Loaded plugin manifest"
                        );
                        manifests.push((manifest, path));
                    },
                    Err(e) => {
                        warn!(
                            path = %manifest_path.display(),
                            error = %e,
                            "Failed to load plugin manifest"
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
                        plugin_id = %manifest.id,
                        "Loaded plugin manifest"
                    );
                    manifests.push((manifest, plugin_dir));
                },
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to load plugin manifest");
                },
            }
        }
    }

    Ok(manifests)
}

/// Load a single plugin manifest from a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn load_manifest(path: &Path) -> PluginResult<PluginManifest> {
    let content = std::fs::read_to_string(path).map_err(|e| PluginError::ManifestParseError {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    let manifest: PluginManifest =
        toml::from_str(&content).map_err(|e| PluginError::ManifestParseError {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    Ok(manifest)
}

/// Plugins directory in a workspace.
#[must_use]
pub fn workspace_plugins_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".astralis").join("plugins")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_manifest_toml() -> &'static str {
        r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
description = "A test plugin"

[entry_point]
type = "wasm"
path = "plugin.wasm"

[[capabilities]]
type = "kv_store"

[[capabilities]]
type = "http_access"
hosts = ["api.example.com"]
"#
    }

    #[test]
    fn test_load_manifest_from_file() {
        let dir = TempDir::new().unwrap();
        let manifest_path = dir.path().join(MANIFEST_FILE_NAME);
        std::fs::write(&manifest_path, sample_manifest_toml()).unwrap();

        let manifest = load_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.id.as_str(), "test-plugin");
        assert_eq!(manifest.name, "Test Plugin");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.description.as_deref(), Some("A test plugin"));
        assert_eq!(manifest.capabilities.len(), 2);
    }

    #[test]
    fn test_load_manifests_from_dir_subdirs() {
        let dir = TempDir::new().unwrap();

        // Create plugin-a/plugin.toml
        let plugin_a = dir.path().join("plugin-a");
        std::fs::create_dir(&plugin_a).unwrap();
        std::fs::write(plugin_a.join(MANIFEST_FILE_NAME), sample_manifest_toml()).unwrap();

        // Create plugin-b/plugin.toml
        let plugin_b = dir.path().join("plugin-b");
        std::fs::create_dir(&plugin_b).unwrap();
        std::fs::write(
            plugin_b.join(MANIFEST_FILE_NAME),
            r#"
id = "other-plugin"
name = "Other Plugin"
version = "1.0.0"

[entry_point]
type = "mcp"
command = "npx"
args = ["-y", "@mcp/server"]
"#,
        )
        .unwrap();

        let results = load_manifests_from_dir(dir.path()).unwrap();
        assert_eq!(results.len(), 2);
        // Each result should include the plugin directory
        for (_, plugin_dir) in &results {
            assert!(plugin_dir.exists());
        }
    }

    #[test]
    fn test_load_manifests_skips_invalid() {
        let dir = TempDir::new().unwrap();

        // Valid plugin
        let valid = dir.path().join("valid");
        std::fs::create_dir(&valid).unwrap();
        std::fs::write(valid.join(MANIFEST_FILE_NAME), sample_manifest_toml()).unwrap();

        // Invalid plugin (bad TOML)
        let invalid = dir.path().join("invalid");
        std::fs::create_dir(&invalid).unwrap();
        std::fs::write(invalid.join(MANIFEST_FILE_NAME), "not valid toml {{{{").unwrap();

        let results = load_manifests_from_dir(dir.path()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id.as_str(), "test-plugin");
    }

    #[test]
    fn test_load_manifest_missing_file() {
        let result = load_manifest(Path::new("/nonexistent/plugin.toml"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::ManifestParseError { .. }
        ));
    }

    #[test]
    fn test_discover_manifests_no_panic() {
        // Should not panic even with no plugins installed
        let manifests = discover_manifests(None);
        let _ = manifests;
    }

    #[test]
    fn test_discover_manifests_with_extra_paths() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("my-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join(MANIFEST_FILE_NAME), sample_manifest_toml()).unwrap();

        let results = discover_manifests(Some(&[dir.path().to_path_buf()]));
        assert!(results.iter().any(|(m, _)| m.id.as_str() == "test-plugin"));
    }

    #[test]
    fn test_workspace_plugins_dir() {
        let dir = workspace_plugins_dir(Path::new("/workspace"));
        assert_eq!(dir, PathBuf::from("/workspace/.astralis/plugins"));
    }
}
