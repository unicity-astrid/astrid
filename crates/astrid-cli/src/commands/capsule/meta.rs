//! Shared capsule installation metadata types and helpers.
//!
//! `CapsuleMeta` is persisted as `meta.json` alongside each installed capsule's
//! `Capsule.toml`. It records the installed version, source, timestamps, and
//! the resolved capability surface (`provides`/`requires`).

use std::fmt;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use astrid_capsule::manifest::TopicDirection;
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
    /// Topic API declarations with inline schema content, baked at install time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) topics: Vec<BakedTopic>,
    /// BLAKE3 hash of the WASM binary, stored content-addressed in `lib/`.
    /// `None` for non-WASM capsules (MCP/OpenClaw).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wasm_hash: Option<String>,
}

/// A topic API declaration baked into `meta.json` with inline schema content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BakedTopic {
    /// The topic name (e.g. `"llm.v1.response.chunk.anthropic"`).
    pub(crate) name: String,
    /// Whether the capsule publishes or subscribes to this topic.
    pub(crate) direction: TopicDirection,
    /// Human-readable description of the topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    /// Inline JSON Schema content (read from the schema file at install time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) schema: Option<serde_json::Value>,
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

    // Principal (user-installed) capsules
    let principal = astrid_core::PrincipalId::default();
    let principal_dir = home.principal_home(&principal).capsules_dir();
    if principal_dir.is_dir() {
        scan_dir(&principal_dir, CapsuleLocation::User, &mut capsules)?;
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

    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "failed to read directory entry, skipping");
                continue;
            },
        };
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to get file type, skipping");
                continue;
            },
        };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = read_meta(&path);
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

    #[test]
    fn baked_topic_round_trip() {
        let meta = CapsuleMeta {
            version: "1.0.0".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            source: None,
            provides: vec![],
            requires: vec![],
            topics: vec![
                BakedTopic {
                    name: "llm.v1.chunk".into(),
                    direction: TopicDirection::Publish,
                    description: Some("Streaming chunk".into()),
                    schema: Some(serde_json::json!({"type": "object"})),
                },
                BakedTopic {
                    name: "llm.v1.request".into(),
                    direction: TopicDirection::Subscribe,
                    description: None,
                    schema: None,
                },
            ],
            wasm_hash: None,
        };

        let json = serde_json::to_string_pretty(&meta).expect("serialize");
        let parsed: CapsuleMeta = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.topics.len(), 2);
        assert_eq!(parsed.topics[0].name, "llm.v1.chunk");
        assert_eq!(parsed.topics[0].direction, TopicDirection::Publish);
        assert_eq!(
            parsed.topics[0].description.as_deref(),
            Some("Streaming chunk")
        );
        assert!(parsed.topics[0].schema.is_some());
        assert_eq!(parsed.topics[1].direction, TopicDirection::Subscribe);
        assert!(parsed.topics[1].schema.is_none());
    }

    #[test]
    fn meta_without_topics_deserializes() {
        // Existing meta.json files without a `topics` field must still parse.
        let json = r#"{"version":"1.0.0","installed_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let meta: CapsuleMeta = serde_json::from_str(json).expect("deserialize");
        assert!(meta.topics.is_empty());
    }

    #[test]
    fn baked_topic_omits_empty_topics_from_json() {
        let meta = CapsuleMeta {
            version: "1.0.0".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            source: None,
            provides: vec![],
            requires: vec![],
            topics: vec![],
            wasm_hash: None,
        };
        let json = serde_json::to_string(&meta).expect("serialize");
        assert!(
            !json.contains("topics"),
            "empty topics should be omitted from JSON"
        );
    }
}
