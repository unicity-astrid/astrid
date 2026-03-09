//! Capsule manifest discovery from standard locations.
//!
//! Scans well-known directories for `Capsule.toml` files, providing
//! the entry point for the Manifest-First architecture.
//!
//! When the `openclaw` feature is enabled, directories containing
//! `openclaw.plugin.json` (but no `Capsule.toml`) are automatically
//! compiled via the OpenClaw pipeline before loading.

use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::CapsuleManifest;

/// Standard capsule manifest file name.
pub const MANIFEST_FILE_NAME: &str = "Capsule.toml";

/// OpenClaw plugin manifest file name.
#[cfg(feature = "openclaw")]
const OPENCLAW_MANIFEST_FILE_NAME: &str = "openclaw.plugin.json";

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
            } else {
                // No Capsule.toml — check for OpenClaw plugin source
                #[cfg(feature = "openclaw")]
                if let Some(result) = try_compile_openclaw(&path) {
                    manifests.push(result);
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

    // Validate ipc_publish and interceptor patterns for empty segments.
    let ipc_patterns = manifest
        .capabilities
        .ipc_publish
        .iter()
        .map(|p| ("ipc_publish pattern", p.as_str()));
    let interceptor_patterns = manifest
        .interceptors
        .iter()
        .map(|i| ("interceptor event pattern", i.event.as_str()));

    for (kind, pattern) in ipc_patterns.chain(interceptor_patterns) {
        if !crate::dispatcher::has_valid_segments(pattern) {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!(
                    "{kind} '{pattern}' contains empty segments \
                     (consecutive dots, leading/trailing dots, or is empty)"
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

/// Detect an OpenClaw plugin directory and compile it to a capsule.
///
/// Returns `Some((manifest, output_dir))` if the directory contains
/// `openclaw.plugin.json` and compilation succeeds. Returns `None` if
/// the directory is not an OpenClaw plugin. Compilation failures are
/// logged as warnings and return `None` (consistent with how manifest
/// parse failures are handled in discovery).
#[cfg(feature = "openclaw")]
fn try_compile_openclaw(plugin_dir: &Path) -> Option<(CapsuleManifest, PathBuf)> {
    use std::collections::HashMap;

    let openclaw_manifest_path = plugin_dir.join(OPENCLAW_MANIFEST_FILE_NAME);
    if !openclaw_manifest_path.exists() {
        return None;
    }

    info!(
        path = %plugin_dir.display(),
        "Detected OpenClaw plugin, compiling"
    );

    // Derive a deterministic output directory from the plugin directory name.
    let output_dir = openclaw_output_dir(plugin_dir)?;

    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        warn!(
            path = %output_dir.display(),
            error = %e,
            "Failed to create OpenClaw output directory"
        );
        return None;
    }

    let cache_dir = astrid_openclaw::pipeline::default_cache_dir();
    let opts = astrid_openclaw::pipeline::CompileOptions {
        plugin_dir,
        output_dir: &output_dir,
        config: &HashMap::new(),
        cache_dir: cache_dir.as_deref(),
        js_only: false,
        no_cache: false,
    };

    match astrid_openclaw::pipeline::compile_plugin(&opts) {
        Ok(result) => {
            info!(
                plugin_id = %result.astrid_id,
                tier = ?result.tier,
                cached = result.cached,
                "OpenClaw plugin compiled successfully"
            );

            // Load the generated Capsule.toml from the output directory
            let manifest_path = output_dir.join(MANIFEST_FILE_NAME);
            match load_manifest(&manifest_path) {
                Ok(manifest) => Some((manifest, output_dir)),
                Err(e) => {
                    warn!(
                        path = %manifest_path.display(),
                        error = %e,
                        "Failed to load generated Capsule.toml from OpenClaw output"
                    );
                    None
                },
            }
        },
        Err(e) => {
            warn!(
                path = %plugin_dir.display(),
                error = %e,
                "Failed to compile OpenClaw plugin"
            );
            None
        },
    }
}

/// Compute a deterministic output directory for a compiled OpenClaw plugin.
///
/// Uses `~/.astrid/cache/openclaw/compiled/{dir_name}/` so that repeated
/// boots reuse the same output location and benefit from the compilation
/// cache.
#[cfg(feature = "openclaw")]
fn openclaw_output_dir(plugin_dir: &Path) -> Option<PathBuf> {
    let dir_name = plugin_dir.file_name()?.to_str()?;
    let base = astrid_openclaw::pipeline::default_cache_dir()?;
    Some(base.join("compiled").join(dir_name))
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

    // ---------------------------------------------------------------
    // OpenClaw integration tests
    // ---------------------------------------------------------------

    #[cfg(feature = "openclaw")]
    mod openclaw {
        use super::*;

        #[test]
        fn output_dir_is_deterministic() {
            let dir = Path::new("/tmp/plugins/my-plugin");
            let result = openclaw_output_dir(dir);
            assert!(result.is_some());
            let path = result.unwrap();
            assert!(
                path.ends_with("compiled/my-plugin"),
                "expected path to end with compiled/my-plugin, got: {}",
                path.display()
            );

            // Calling again produces the same path
            let second = openclaw_output_dir(dir).unwrap();
            assert_eq!(path, second, "output dir must be deterministic");
        }

        #[test]
        fn try_compile_returns_none_for_non_openclaw_dir() {
            let dir = tempfile::tempdir().unwrap();
            // Empty directory — no openclaw.plugin.json
            let result = try_compile_openclaw(dir.path());
            assert!(result.is_none(), "non-OpenClaw dir should return None");
        }

        #[test]
        fn try_compile_returns_none_for_corrupt_manifest() {
            let dir = tempfile::tempdir().unwrap();
            // Write invalid JSON to openclaw.plugin.json
            std::fs::write(
                dir.path().join(OPENCLAW_MANIFEST_FILE_NAME),
                "not valid json {{{",
            )
            .unwrap();
            let result = try_compile_openclaw(dir.path());
            assert!(
                result.is_none(),
                "corrupt manifest should return None (logged as warning)"
            );
        }

        #[test]
        fn try_compile_returns_none_for_missing_entry_point() {
            let dir = tempfile::tempdir().unwrap();
            // Valid JSON manifest but no source files
            std::fs::write(
                dir.path().join(OPENCLAW_MANIFEST_FILE_NAME),
                r#"{"id": "test-plugin", "configSchema": {}}"#,
            )
            .unwrap();
            let result = try_compile_openclaw(dir.path());
            assert!(
                result.is_none(),
                "missing entry point should return None (logged as warning)"
            );
        }

        #[test]
        fn load_manifests_prefers_capsule_toml_over_openclaw() {
            let root = tempfile::tempdir().unwrap();
            let plugin_dir = root.path().join("my-plugin");
            std::fs::create_dir(&plugin_dir).unwrap();

            // Write both Capsule.toml and openclaw.plugin.json
            std::fs::write(
                plugin_dir.join("Capsule.toml"),
                "[package]\nname = \"precompiled\"\nversion = \"1.0.0\"\n",
            )
            .unwrap();
            std::fs::write(
                plugin_dir.join(OPENCLAW_MANIFEST_FILE_NAME),
                r#"{"id": "precompiled", "configSchema": {}}"#,
            )
            .unwrap();

            let manifests = load_manifests_from_dir(root.path()).unwrap();
            assert_eq!(manifests.len(), 1, "should load exactly one manifest");
            assert_eq!(
                manifests[0].0.package.name, "precompiled",
                "should load the existing Capsule.toml, not re-compile"
            );
            // The capsule_dir should be the original plugin dir, not a compiled output
            assert_eq!(manifests[0].1, plugin_dir);
        }
    }
}
