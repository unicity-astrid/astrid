//! Serde types for npm registry JSON responses.

use std::collections::HashMap;

use serde::Deserialize;

/// Top-level npm registry response for a package.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageMetadata {
    /// Package name.
    #[serde(default)]
    pub name: String,
    /// Mapping of dist-tags to version strings (e.g. `{"latest": "1.0.0"}`).
    #[serde(rename = "dist-tags", default)]
    pub dist_tags: HashMap<String, String>,
    /// Per-version metadata, keyed by semver string.
    #[serde(default)]
    pub versions: HashMap<String, VersionMetadata>,
}

/// Metadata for a single package version.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionMetadata {
    /// Package name.
    #[serde(default)]
    pub name: String,
    /// Version string.
    #[serde(default)]
    pub version: String,
    /// Distribution info (tarball URL, integrity hashes).
    pub dist: DistInfo,
}

/// Distribution information for a published version.
#[derive(Debug, Clone, Deserialize)]
pub struct DistInfo {
    /// Tarball download URL.
    pub tarball: String,
    /// SRI integrity string (e.g. `"sha512-<base64>"`).
    #[serde(default)]
    pub integrity: Option<String>,
    /// SHA-1 hex digest (legacy fallback).
    #[serde(default)]
    pub shasum: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_package_metadata() {
        let json = r#"{
            "name": "@openclaw/hello-tool",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "@openclaw/hello-tool",
                    "version": "1.0.0",
                    "dist": {
                        "tarball": "https://registry.npmjs.org/@openclaw/hello-tool/-/hello-tool-1.0.0.tgz",
                        "integrity": "sha512-abc123==",
                        "shasum": "deadbeef"
                    }
                }
            }
        }"#;

        let meta: PackageMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(meta.name, "@openclaw/hello-tool");
        assert_eq!(meta.dist_tags.get("latest").unwrap(), "1.0.0");

        let v = meta.versions.get("1.0.0").unwrap();
        assert_eq!(v.version, "1.0.0");
        assert_eq!(
            v.dist.tarball,
            "https://registry.npmjs.org/@openclaw/hello-tool/-/hello-tool-1.0.0.tgz"
        );
        assert_eq!(v.dist.integrity.as_deref(), Some("sha512-abc123=="));
        assert_eq!(v.dist.shasum.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn deserialize_minimal_version() {
        let json = r#"{
            "name": "simple",
            "version": "0.1.0",
            "dist": {
                "tarball": "https://registry.npmjs.org/simple/-/simple-0.1.0.tgz"
            }
        }"#;

        let v: VersionMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(v.name, "simple");
        assert_eq!(v.version, "0.1.0");
        assert!(v.dist.integrity.is_none());
        assert!(v.dist.shasum.is_none());
    }

    #[test]
    fn deserialize_empty_dist_tags() {
        let json = r#"{
            "name": "test",
            "versions": {}
        }"#;

        let meta: PackageMetadata = serde_json::from_str(json).unwrap();
        assert!(meta.dist_tags.is_empty());
        assert!(meta.versions.is_empty());
    }
}
