//! Capsule manifest discovery from standard locations.
//!
//! Scans well-known directories for `Capsule.toml` files, providing
//! the entry point for the Manifest-First architecture.

use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::CapsuleManifest;

/// Standard capsule manifest file name.
pub(crate) const MANIFEST_FILE_NAME: &str = "Capsule.toml";

/// Discover capsule manifests from standard locations.
///
/// Scans the following directories for `Capsule.toml` files:
/// 1. `.astrid/capsules/` (workspace-level, relative to CWD)
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
pub(crate) fn load_manifests_from_dir(
    dir: &Path,
) -> CapsuleResult<Vec<(CapsuleManifest, PathBuf)>> {
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
            let capsule_dir = path.parent().unwrap_or(dir).to_path_buf();
            match load_manifest(&path) {
                Ok(manifest) => {
                    debug!(
                        path = %path.display(),
                        capsule_name = %manifest.package.name,
                        "Loaded capsule manifest"
                    );
                    manifests.push((manifest, capsule_dir));
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

    // Validate version is valid semver (same as Cargo.toml).
    if semver::Version::parse(&manifest.package.version).is_err() {
        return Err(CapsuleError::ManifestParseError {
            path: path.to_path_buf(),
            message: format!(
                "invalid version '{}' in [package] - must be valid semver (MAJOR.MINOR.PATCH)",
                manifest.package.version
            ),
        });
    }

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

    // Validate dependency capability strings.
    let dep_caps = manifest
        .dependencies
        .provides
        .iter()
        .map(|c| ("provides", c.as_str()))
        .chain(
            manifest
                .dependencies
                .requires
                .iter()
                .map(|c| ("requires", c.as_str())),
        );

    for (kind, cap) in dep_caps {
        if cap.is_empty() {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!("[dependencies].{kind} contains an empty capability string"),
            });
        }
        let Some((prefix, body)) = cap.split_once(':') else {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!(
                    "[dependencies].{kind} '{cap}' must have a type prefix \
                     (e.g. topic:, tool:, llm:, uplink:)"
                ),
            });
        };
        const KNOWN_PREFIXES: &[&str] = &["topic", "tool", "llm", "uplink"];
        if !KNOWN_PREFIXES.contains(&prefix) {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!(
                    "[dependencies].{kind} '{cap}' has unknown prefix '{prefix}:' \
                     (expected one of: topic:, tool:, llm:, uplink:)"
                ),
            });
        }
        if !crate::dispatcher::has_valid_segments(body) {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!(
                    "[dependencies].{kind} '{cap}' body contains empty segments \
                     (consecutive dots, leading/trailing dots, or is empty)"
                ),
            });
        }
        // Wildcards are only valid in `requires` (pattern matching).
        // In `provides`, a capsule must declare concrete capabilities.
        if kind == "provides" && body.contains('*') {
            return Err(CapsuleError::ManifestParseError {
                path: path.to_path_buf(),
                message: format!(
                    "[dependencies].provides '{cap}' contains a wildcard - \
                     provides must be concrete capabilities, not patterns"
                ),
            });
        }
    }

    // Uplink capsules load in a partition before non-uplinks.
    // Declaring `requires` on an uplink would violate this ordering.
    if manifest.capabilities.uplink && !manifest.dependencies.requires.is_empty() {
        return Err(CapsuleError::ManifestParseError {
            path: path.to_path_buf(),
            message: "[dependencies].requires is not allowed on uplink capsules \
                      (uplinks load before non-uplinks and cannot depend on them)"
                .into(),
        });
    }

    Ok(manifest)
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

    #[test]
    fn load_manifest_accepts_valid_semver() {
        let toml = "[package]\nname = \"test\"\nversion = \"1.2.3\"\n";
        assert!(load_from_toml(toml).is_ok());
    }

    #[test]
    fn load_manifest_accepts_prerelease_semver() {
        let toml = "[package]\nname = \"test\"\nversion = \"1.0.0-alpha.1\"\n";
        assert!(load_from_toml(toml).is_ok());
    }

    #[test]
    fn load_manifest_rejects_incomplete_semver() {
        let toml = "[package]\nname = \"test\"\nversion = \"1.0\"\n";
        let err = load_from_toml(toml).unwrap_err();
        assert!(
            err.to_string().contains("invalid version"),
            "expected 'invalid version' error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_non_semver_version() {
        let toml = "[package]\nname = \"test\"\nversion = \"latest\"\n";
        let err = load_from_toml(toml).unwrap_err();
        assert!(
            err.to_string().contains("invalid version"),
            "expected 'invalid version' error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_parses_dependencies_provides_requires() {
        let toml = format!(
            "{VALID_HEADER}\n\
             [dependencies]\n\
             provides = [\"topic:identity.response.ready\"]\n\
             requires = [\"topic:llm.stream.*\"]\n"
        );
        let m = load_from_toml(&toml).unwrap();
        assert_eq!(
            m.dependencies.provides,
            vec!["topic:identity.response.ready"]
        );
        assert_eq!(m.dependencies.requires, vec!["topic:llm.stream.*"]);
    }

    #[test]
    fn load_manifest_defaults_empty_dependencies() {
        let m = load_from_toml(VALID_HEADER).unwrap();
        assert!(m.dependencies.provides.is_empty());
        assert!(m.dependencies.requires.is_empty());
        assert!(m.dependencies.is_empty());
    }

    #[test]
    fn load_manifest_parses_dependencies_provides_only() {
        let toml = format!(
            "{VALID_HEADER}\n\
             [dependencies]\n\
             provides = [\"topic:foo\", \"tool:bar\"]\n"
        );
        let m = load_from_toml(&toml).unwrap();
        assert_eq!(m.dependencies.provides, vec!["topic:foo", "tool:bar"]);
        assert!(m.dependencies.requires.is_empty());
    }

    #[test]
    fn load_manifest_rejects_empty_capability_in_requires() {
        let toml = format!("{VALID_HEADER}\n[dependencies]\nrequires = [\"\"]");
        let err = load_from_toml(&toml).unwrap_err();
        assert!(
            err.to_string().contains("empty capability string"),
            "expected 'empty capability string' error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_missing_prefix_in_provides() {
        let toml = format!("{VALID_HEADER}\n[dependencies]\nprovides = [\"no_prefix\"]");
        let err = load_from_toml(&toml).unwrap_err();
        assert!(
            err.to_string().contains("must have a type prefix"),
            "expected 'must have a type prefix' error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_unknown_prefix_in_requires() {
        let toml = format!("{VALID_HEADER}\n[dependencies]\nrequires = [\"service:foo\"]");
        let err = load_from_toml(&toml).unwrap_err();
        assert!(
            err.to_string().contains("unknown prefix"),
            "expected 'unknown prefix' error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_empty_segments_in_dependency_body() {
        let toml = format!("{VALID_HEADER}\n[dependencies]\nprovides = [\"topic:a..b\"]");
        let err = load_from_toml(&toml).unwrap_err();
        assert!(
            err.to_string().contains("empty segments"),
            "expected 'empty segments' error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_wildcard_in_provides() {
        let toml = format!("{VALID_HEADER}\n[dependencies]\nprovides = [\"topic:llm.stream.*\"]");
        let err = load_from_toml(&toml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("wildcard"),
            "expected 'wildcard' error for provides with *, got: {msg}"
        );
    }

    #[test]
    fn load_manifest_allows_wildcard_in_requires() {
        let toml = format!("{VALID_HEADER}\n[dependencies]\nrequires = [\"topic:llm.stream.*\"]");
        assert!(
            load_from_toml(&toml).is_ok(),
            "wildcards should be allowed in requires"
        );
    }

    #[test]
    fn load_manifest_rejects_uplink_with_requires() {
        let toml = format!(
            "{VALID_HEADER}\n[capabilities]\nuplink = true\n\n[dependencies]\nrequires = [\"topic:foo\"]"
        );
        let err = load_from_toml(&toml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not allowed on uplink"),
            "expected uplink+requires rejection, got: {msg}"
        );
    }

    #[test]
    fn load_manifest_allows_uplink_without_requires() {
        let toml = format!("{VALID_HEADER}\n[capabilities]\nuplink = true");
        assert!(
            load_from_toml(&toml).is_ok(),
            "uplink without requires should be valid"
        );
    }
}
