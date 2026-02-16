//! `OpenClaw` manifest parsing and ID conversion.
//!
//! Reads `openclaw.plugin.json` from a plugin directory, validates required
//! fields, and converts the `OpenClaw` ID to an Astrid-compatible `PluginId`
//! (lowercase, hyphens only, no leading/trailing hyphens).
//!
//! Entry points are NOT stored in `openclaw.plugin.json`. They come from
//! `package.json` → `openclaw.extensions` array. Use [`resolve_entry_point`]
//! to find the plugin's main file.

use std::path::Path;

use serde::Deserialize;

use crate::error::{BridgeError, BridgeResult};

/// Parsed `OpenClaw` plugin manifest (`openclaw.plugin.json`).
///
/// Real `OpenClaw` manifests have `id` and `configSchema` as required fields.
/// Everything else (`name`, `version`, `description`, `kind`, `channels`,
/// `providers`, `skills`) is optional.
///
/// Entry points are NOT in this file — they come from `package.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawManifest {
    /// Plugin identifier (e.g. `"hello-tool"`, `"my_plugin.v2"`).
    pub id: String,
    /// JSON Schema for plugin configuration.
    pub config_schema: serde_json::Value,
    /// Optional human-readable name.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional semantic version string.
    #[serde(default)]
    pub version: Option<String>,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional plugin kind (e.g. `"tool"`, `"service"`).
    #[serde(default)]
    pub kind: Option<String>,
    /// Channel identifiers this plugin registers (e.g. `["discord"]`).
    #[serde(default)]
    pub channels: Vec<String>,
    /// Provider identifiers this plugin registers.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Skill identifiers this plugin registers.
    #[serde(default)]
    pub skills: Vec<String>,
}

impl OpenClawManifest {
    /// Get the plugin name, falling back to the ID if not specified.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// Get the plugin version, falling back to `"0.0.0"` if not specified.
    #[must_use]
    pub fn display_version(&self) -> &str {
        self.version.as_deref().unwrap_or("0.0.0")
    }

    /// Whether this plugin declares channels or providers that require
    /// host-side integration (Tier 2 / Node.js).
    #[must_use]
    pub fn requires_host_integration(&self) -> bool {
        !self.channels.is_empty() || !self.providers.is_empty()
    }
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

/// Validate required fields.
fn validate_manifest(m: &OpenClawManifest) -> BridgeResult<()> {
    if m.id.is_empty() {
        return Err(BridgeError::Manifest("'id' must not be empty".into()));
    }
    Ok(())
}

/// Resolve the plugin's entry point file from `package.json`.
///
/// Looks for the entry point in this order:
/// 1. `package.json` → `openclaw.extensions[0]`
/// 2. `src/index.ts`
/// 3. `src/index.js`
/// 4. `index.ts`
/// 5. `index.js`
///
/// # Errors
///
/// Returns `BridgeError::Manifest` if no entry point can be found.
pub fn resolve_entry_point(plugin_dir: &Path) -> BridgeResult<String> {
    // Try package.json → openclaw.extensions
    let pkg_path = plugin_dir.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_path)
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content)
        && let Some(extensions) = parsed
            .get("openclaw")
            .and_then(|oc| oc.get("extensions"))
            .and_then(|e| e.as_array())
        && let Some(first) = extensions.first().and_then(|v| v.as_str())
    {
        return Ok(first.to_string());
    }

    // Fallback: check common entry point locations
    let candidates = ["src/index.ts", "src/index.js", "index.ts", "index.js"];
    for candidate in &candidates {
        if plugin_dir.join(candidate).exists() {
            return Ok((*candidate).to_string());
        }
    }

    Err(BridgeError::Manifest(
        "could not resolve entry point: no 'openclaw.extensions' in package.json \
         and no src/index.ts, src/index.js, index.ts, or index.js found"
            .into(),
    ))
}

/// Convert an `OpenClaw` plugin ID to an Astrid-compatible plugin ID.
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
    fn parse_manifest_minimal() {
        let dir = std::env::temp_dir().join("oc-bridge-test-minimal");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"{"id":"hello-tool","configSchema":{"type":"object","properties":{}}}"#,
        )
        .unwrap();

        let m = parse_manifest(&dir).unwrap();
        assert_eq!(m.id, "hello-tool");
        assert!(m.name.is_none());
        assert!(m.version.is_none());
        assert_eq!(m.display_name(), "hello-tool");
        assert_eq!(m.display_version(), "0.0.0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_manifest_full() {
        let dir = std::env::temp_dir().join("oc-bridge-test-full");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"{
                "id": "discord",
                "name": "Discord Channel",
                "version": "1.0.0",
                "description": "Discord integration",
                "configSchema": {"type":"object","properties":{}},
                "channels": ["discord"],
                "providers": []
            }"#,
        )
        .unwrap();

        let m = parse_manifest(&dir).unwrap();
        assert_eq!(m.id, "discord");
        assert_eq!(m.name.as_deref(), Some("Discord Channel"));
        assert_eq!(m.channels, vec!["discord"]);
        assert!(m.requires_host_integration());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_manifest_with_providers() {
        let dir = std::env::temp_dir().join("oc-bridge-test-providers");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"{"id":"copilot-proxy","configSchema":{},"providers":["copilot-proxy"]}"#,
        )
        .unwrap();

        let m = parse_manifest(&dir).unwrap();
        assert!(m.requires_host_integration());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_manifest_missing_id() {
        let dir = std::env::temp_dir().join("oc-bridge-test-no-id");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(MANIFEST_FILENAME), r#"{"configSchema":{}}"#).unwrap();

        let result = parse_manifest(&dir);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_manifest_missing_config_schema() {
        let dir = std::env::temp_dir().join("oc-bridge-test-no-schema");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(MANIFEST_FILENAME), r#"{"id":"test"}"#).unwrap();

        let result = parse_manifest(&dir);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_entry_from_package_json() {
        let dir = std::env::temp_dir().join("oc-bridge-test-entry-pkg");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("package.json"),
            r#"{"openclaw":{"extensions":["./src/index.ts"]}}"#,
        )
        .unwrap();

        let entry = resolve_entry_point(&dir).unwrap();
        assert_eq!(entry, "./src/index.ts");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_entry_fallback_src_index_ts() {
        let dir = std::env::temp_dir().join("oc-bridge-test-entry-fallback");
        let _ = std::fs::create_dir_all(dir.join("src"));
        std::fs::write(dir.join("src/index.ts"), "// plugin").unwrap();

        let entry = resolve_entry_point(&dir).unwrap();
        assert_eq!(entry, "src/index.ts");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_entry_no_match_fails() {
        let dir = std::env::temp_dir().join("oc-bridge-test-entry-none");
        let _ = std::fs::create_dir_all(&dir);

        let result = resolve_entry_point(&dir);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_host_integration_for_simple_plugin() {
        let m = OpenClawManifest {
            id: "simple-tool".into(),
            config_schema: serde_json::json!({}),
            name: None,
            version: None,
            description: None,
            kind: None,
            channels: vec![],
            providers: vec![],
            skills: vec![],
        };
        assert!(!m.requires_host_integration());
    }
}
