//! `OpenClaw` manifest parsing and ID conversion.
//!
//! Reads `openclaw.plugin.json` from a plugin directory, validates required
//! fields, and converts the `OpenClaw` ID to an Astralis-compatible `PluginId`
//! (lowercase, hyphens only, no leading/trailing hyphens).

use std::path::Path;

use serde::Deserialize;

use crate::error::{BridgeError, BridgeResult};

/// Parsed `OpenClaw` plugin manifest (`openclaw.plugin.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct OpenClawManifest {
    /// Plugin identifier (e.g. `"hello-tool"`, `"my_plugin.v2"`).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Entry point file relative to the plugin directory.
    pub main: String,
    /// Engine version constraints.
    #[serde(default)]
    pub engines: Option<EngineConstraints>,
}

/// Engine version constraints from the manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct EngineConstraints {
    /// Required `OpenClaw` engine version (e.g. `"^0.x"`).
    #[serde(default)]
    pub openclaw: Option<String>,
}

const MANIFEST_FILENAME: &str = "openclaw.plugin.json";

/// Parse the `OpenClaw` manifest from a plugin directory.
///
/// # Errors
///
/// Returns `BridgeError::Manifest` if the file cannot be read, parsed, or fails validation.
pub fn parse_manifest(plugin_dir: &Path) -> BridgeResult<OpenClawManifest> {
    let manifest_path = plugin_dir.join(MANIFEST_FILENAME);
    let content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        BridgeError::Manifest(format!("failed to read {}: {e}", manifest_path.display()))
    })?;

    let manifest: OpenClawManifest = serde_json::from_str(&content)
        .map_err(|e| BridgeError::Manifest(format!("failed to parse {MANIFEST_FILENAME}: {e}")))?;

    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Validate required fields and engine constraints.
fn validate_manifest(m: &OpenClawManifest) -> BridgeResult<()> {
    if m.id.is_empty() {
        return Err(BridgeError::Manifest("'id' must not be empty".into()));
    }
    if m.name.is_empty() {
        return Err(BridgeError::Manifest("'name' must not be empty".into()));
    }
    if m.version.is_empty() {
        return Err(BridgeError::Manifest("'version' must not be empty".into()));
    }
    if m.main.is_empty() {
        return Err(BridgeError::Manifest("'main' must not be empty".into()));
    }

    // Check engine version if specified — we only support ^0.x
    if let Some(engines) = &m.engines
        && let Some(oc_version) = &engines.openclaw
        && !oc_version.starts_with("^0.")
        && !oc_version.starts_with("0.")
    {
        return Err(BridgeError::Manifest(format!(
            "unsupported engine version '{oc_version}', expected ^0.x"
        )));
    }

    Ok(())
}

/// Convert an `OpenClaw` plugin ID to an Astralis-compatible plugin ID.
///
/// Rules:
/// - Lowercase the entire string
/// - Replace `_` and `.` with `-`
/// - Collapse consecutive hyphens
/// - Strip leading/trailing hyphens
/// - Validate: `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`
///
/// # Errors
///
/// Returns `BridgeError::InvalidId` if the converted ID is empty or contains invalid characters.
pub fn convert_id(openclaw_id: &str) -> BridgeResult<String> {
    let mut id: String = openclaw_id
        .to_lowercase()
        .chars()
        .map(|c| if c == '_' || c == '.' { '-' } else { c })
        .collect();

    // Collapse consecutive hyphens
    while id.contains("--") {
        id = id.replace("--", "-");
    }

    // Strip leading/trailing hyphens
    let id = id.trim_matches('-').to_string();

    // Validate the result
    if id.is_empty() {
        return Err(BridgeError::InvalidId {
            original: openclaw_id.into(),
            reason: "converted id is empty".into(),
        });
    }

    let valid = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(BridgeError::InvalidId {
            original: openclaw_id.into(),
            reason: format!("contains invalid characters after conversion: '{id}'"),
        });
    }

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_id_simple() {
        assert_eq!(convert_id("hello-tool").unwrap(), "hello-tool");
    }

    #[test]
    fn convert_id_underscore() {
        assert_eq!(convert_id("my_plugin").unwrap(), "my-plugin");
    }

    #[test]
    fn convert_id_dots() {
        assert_eq!(convert_id("my.plugin.v2").unwrap(), "my-plugin-v2");
    }

    #[test]
    fn convert_id_uppercase() {
        assert_eq!(convert_id("MyPlugin").unwrap(), "myplugin");
    }

    #[test]
    fn convert_id_mixed() {
        assert_eq!(convert_id("My_Cool.Plugin").unwrap(), "my-cool-plugin");
    }

    #[test]
    fn convert_id_leading_trailing_special() {
        assert_eq!(convert_id("_plugin_").unwrap(), "plugin");
    }

    #[test]
    fn convert_id_consecutive_separators() {
        assert_eq!(convert_id("a__b..c").unwrap(), "a-b-c");
    }

    #[test]
    fn convert_id_empty_fails() {
        assert!(convert_id("").is_err());
    }

    #[test]
    fn convert_id_only_separators_fails() {
        assert!(convert_id("___").is_err());
    }

    #[test]
    fn convert_id_special_chars_fail() {
        assert!(convert_id("plugin@v1").is_err());
    }

    #[test]
    fn parse_manifest_valid() {
        let dir = std::env::temp_dir().join("oc-bridge-test-valid");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"{"id":"hello-tool","name":"Hello Tool","version":"1.0.0","main":"index.js"}"#,
        )
        .unwrap();

        let m = parse_manifest(&dir).unwrap();
        assert_eq!(m.id, "hello-tool");
        assert_eq!(m.name, "Hello Tool");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.main, "index.js");
        assert!(m.engines.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_manifest_with_engines() {
        let dir = std::env::temp_dir().join("oc-bridge-test-engines");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"{"id":"test","name":"Test","version":"1.0.0","main":"index.js","engines":{"openclaw":"^0.1"}}"#,
        )
        .unwrap();

        let m = parse_manifest(&dir).unwrap();
        assert_eq!(
            m.engines.as_ref().unwrap().openclaw.as_deref(),
            Some("^0.1")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_manifest_bad_engine_version() {
        let dir = std::env::temp_dir().join("oc-bridge-test-bad-engine");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"{"id":"test","name":"Test","version":"1.0.0","main":"index.js","engines":{"openclaw":"^2.0"}}"#,
        )
        .unwrap();

        let result = parse_manifest(&dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unsupported engine version"), "got: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_manifest_missing_field() {
        let dir = std::env::temp_dir().join("oc-bridge-test-missing");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"{"id":"test","name":"Test"}"#,
        )
        .unwrap();

        let result = parse_manifest(&dir);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
