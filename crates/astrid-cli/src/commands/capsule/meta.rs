//! Shared capsule installation metadata types and helpers.
//!
//! `CapsuleMeta` is persisted as `meta.json` alongside each installed capsule's
//! `Capsule.toml`. It records the installed version, source, timestamps, and
//! the resolved capability surface (`provides`/`requires`).

use std::fmt;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use astrid_core::dirs::AstridHome;

/// Capsule installation metadata, persisted as `meta.json` alongside `Capsule.toml`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CapsuleMeta {
    /// The currently installed version.
    pub(crate) version: String,
    /// When the capsule was first installed.
    pub(crate) installed_at: String,
    /// When the capsule was last updated.
    pub(crate) updated_at: String,
    /// The original install source (local path, GitHub URL, openclaw: prefix, etc.).
    /// Used by `astrid capsule update` to re-fetch from the same source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    /// Resolved capabilities this capsule provides (baked from `effective_provides()`
    /// at install time so registries and tooling can read them without running Rust).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) provides: Vec<String>,
    /// Capabilities this capsule requires from other capsules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) requires: Vec<String>,
}

/// Read existing `meta.json` from a capsule's install directory (if present).
pub(crate) fn read_meta(target_dir: &Path) -> Option<CapsuleMeta> {
    let meta_path = target_dir.join("meta.json");
    let data = match std::fs::read_to_string(&meta_path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(
                path = %meta_path.display(),
                error = %e,
                "failed to read meta.json, treating as missing"
            );
            return None;
        },
    };
    match serde_json::from_str::<CapsuleMeta>(&data) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(
                path = %meta_path.display(),
                error = %e,
                "meta.json is corrupt, treating as missing"
            );
            None
        },
    }
}

/// Write `meta.json` to the capsule's install directory.
///
/// Uses atomic write (temp file + rename) to avoid corruption from
/// crashes or power loss during write.
pub(crate) fn write_meta(target_dir: &Path, meta: &CapsuleMeta) -> anyhow::Result<()> {
    let meta_path = target_dir.join("meta.json");
    let json = serde_json::to_string_pretty(meta).context("failed to serialize meta.json")?;
    let mut tmp = tempfile::NamedTempFile::new_in(target_dir)
        .context("failed to create temp file for meta.json")?;
    std::io::Write::write_all(&mut tmp, json.as_bytes())
        .context("failed to write meta.json staging")?;
    tmp.persist(&meta_path)
        .map_err(|e| anyhow::anyhow!("failed to persist {}: {e}", meta_path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scanning helpers
// ---------------------------------------------------------------------------

/// Where an installed capsule lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapsuleLocation {
    /// User-level: `~/.astrid/capsules/`
    User,
    /// Workspace-level: `.astrid/capsules/` relative to CWD
    Workspace,
}

impl fmt::Display for CapsuleLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Workspace => f.write_str("workspace"),
        }
    }
}

/// An installed capsule discovered on disk.
pub(crate) struct InstalledCapsule {
    /// Directory name (capsule ID).
    pub(crate) name: String,
    /// Parsed `meta.json`, if present.
    pub(crate) meta: Option<CapsuleMeta>,
    /// Where this capsule was found.
    pub(crate) location: CapsuleLocation,
}

/// Scan user-level and workspace capsule directories, returning all installed
/// capsules sorted alphabetically by name.
pub(crate) fn scan_installed_capsules() -> anyhow::Result<Vec<InstalledCapsule>> {
    let home = AstridHome::resolve().context("failed to resolve Astrid home directory")?;
    let mut capsules = Vec::new();

    // User-level capsules
    let user_dir = home.capsules_dir();
    if user_dir.is_dir() {
        scan_dir(&user_dir, CapsuleLocation::User, &mut capsules)?;
    }

    // Workspace-level capsules
    if let Ok(cwd) = std::env::current_dir() {
        let ws_dir = cwd.join(".astrid").join("capsules");
        if ws_dir.is_dir() {
            scan_dir(&ws_dir, CapsuleLocation::Workspace, &mut capsules)?;
        }
    }

    capsules.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(capsules)
}

/// Scan a single directory for capsule subdirectories.
fn scan_dir(
    dir: &Path,
    location: CapsuleLocation,
    out: &mut Vec<InstalledCapsule>,
) -> anyhow::Result<()> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let ft = entry.file_type()?;
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = read_meta(&entry.path());
        out.push(InstalledCapsule {
            name,
            meta,
            location,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_installed_capsules_with_meta() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let capsules_dir = tmp.path().join("capsules");
        std::fs::create_dir_all(&capsules_dir).expect("mkdir");

        // Create two capsule directories with meta.json
        let cap_a = capsules_dir.join("alpha");
        std::fs::create_dir_all(&cap_a).expect("mkdir");
        std::fs::write(
            cap_a.join("meta.json"),
            r#"{"version":"1.0.0","installed_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","provides":["topic:foo"],"requires":["topic:bar"]}"#,
        )
        .expect("write");

        let cap_b = capsules_dir.join("bravo");
        std::fs::create_dir_all(&cap_b).expect("mkdir");
        std::fs::write(
            cap_b.join("meta.json"),
            r#"{"version":"2.0.0","installed_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#,
        )
        .expect("write");

        let mut results = Vec::new();
        scan_dir(&capsules_dir, CapsuleLocation::User, &mut results).expect("scan");

        results.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(results.len(), 2);

        assert_eq!(results[0].name, "alpha");
        let meta_a = results[0].meta.as_ref().expect("alpha has meta");
        assert_eq!(meta_a.version, "1.0.0");
        assert_eq!(meta_a.provides, vec!["topic:foo"]);
        assert_eq!(meta_a.requires, vec!["topic:bar"]);

        assert_eq!(results[1].name, "bravo");
        let meta_b = results[1].meta.as_ref().expect("bravo has meta");
        assert_eq!(meta_b.version, "2.0.0");
        assert!(meta_b.provides.is_empty());
        assert!(meta_b.requires.is_empty());
    }

    #[test]
    fn test_scan_empty_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let capsules_dir = tmp.path().join("capsules");
        std::fs::create_dir_all(&capsules_dir).expect("mkdir");

        let mut results = Vec::new();
        scan_dir(&capsules_dir, CapsuleLocation::User, &mut results).expect("scan");
        assert!(results.is_empty());
    }

    #[test]
    fn test_scan_missing_meta() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let capsules_dir = tmp.path().join("capsules");
        let cap = capsules_dir.join("no-meta-capsule");
        std::fs::create_dir_all(&cap).expect("mkdir");

        let mut results = Vec::new();
        scan_dir(&capsules_dir, CapsuleLocation::User, &mut results).expect("scan");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "no-meta-capsule");
        assert!(results[0].meta.is_none());
    }

    #[test]
    fn test_scan_corrupt_meta_returns_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let capsules_dir = tmp.path().join("capsules");
        let cap = capsules_dir.join("corrupt-capsule");
        std::fs::create_dir_all(&cap).expect("mkdir");
        // Write invalid JSON
        std::fs::write(cap.join("meta.json"), "{{not valid json").expect("write");

        let mut results = Vec::new();
        scan_dir(&capsules_dir, CapsuleLocation::User, &mut results).expect("scan");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "corrupt-capsule");
        assert!(
            results[0].meta.is_none(),
            "corrupt meta.json should be treated as missing"
        );
    }
}
