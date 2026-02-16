//! Astrid plugin manifest (`plugin.toml`) generation and output assembly.
//!
//! Generates a `plugin.toml` that is serde-compatible with
//! [`astrid_plugins::manifest::PluginManifest`] including the tagged enum
//! format for `entry_point` and `capabilities`.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{BridgeError, BridgeResult};
use crate::manifest::OpenClawManifest;

/// A serializable Astrid plugin manifest matching the `PluginManifest` serde format.
///
/// This is a local mirror — we don't depend on `astrid-plugins` at compile time
/// to keep the binary lean. The dev-dependency round-trip test verifies compatibility.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OutputManifest {
    id: String,
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    entry_point: OutputEntryPoint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    capabilities: Vec<OutputCapability>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    config: HashMap<String, serde_json::Value>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputEntryPoint {
    Wasm {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        hash: Option<String>,
    },
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputCapability {
    Config,
}

/// Generate `plugin.toml` in the output directory.
///
/// Reads the compiled WASM file to compute a blake3 hash, then writes
/// the manifest with the correct `PluginManifest` serde format.
///
/// # Errors
///
/// Returns `BridgeError::Output` if the WASM file cannot be read or the TOML cannot be written.
#[allow(clippy::implicit_hasher)]
pub fn generate_manifest(
    astrid_id: &str,
    oc_manifest: &OpenClawManifest,
    wasm_path: &Path,
    config: &HashMap<String, serde_json::Value>,
    output_dir: &Path,
) -> BridgeResult<()> {
    // Compute blake3 hash of the WASM file
    let wasm_bytes = std::fs::read(wasm_path)
        .map_err(|e| BridgeError::Output(format!("failed to read WASM file for hashing: {e}")))?;
    let hash = blake3::hash(&wasm_bytes).to_hex().to_string();

    // Build capabilities
    let mut capabilities = Vec::new();
    if !config.is_empty() {
        capabilities.push(OutputCapability::Config);
    }

    let manifest = OutputManifest {
        id: astrid_id.to_string(),
        name: oc_manifest.display_name().to_string(),
        version: oc_manifest.display_version().to_string(),
        description: oc_manifest.description.clone(),
        author: None,
        entry_point: OutputEntryPoint::Wasm {
            path: "plugin.wasm".into(),
            hash: Some(hash),
        },
        capabilities,
        config: config.clone(),
    };

    let toml_str = toml::to_string_pretty(&manifest)
        .map_err(|e| BridgeError::Output(format!("failed to serialize plugin.toml: {e}")))?;

    let toml_path = output_dir.join("plugin.toml");
    std::fs::write(&toml_path, toml_str)
        .map_err(|e| BridgeError::Output(format!("failed to write plugin.toml: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn output_manifest_serializes_correctly() {
        let manifest = OutputManifest {
            id: "hello-tool".into(),
            name: "Hello Tool".into(),
            version: "1.0.0".into(),
            description: Some("A test plugin".into()),
            author: None,
            entry_point: OutputEntryPoint::Wasm {
                path: "plugin.wasm".into(),
                hash: Some("abc123".into()),
            },
            capabilities: vec![OutputCapability::Config],
            config: HashMap::from([("timeout".into(), serde_json::json!(30))]),
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        assert!(toml_str.contains("id = \"hello-tool\""));
        assert!(toml_str.contains("type = \"wasm\""));
        assert!(toml_str.contains("path = \"plugin.wasm\""));
        assert!(toml_str.contains("hash = \"abc123\""));
        assert!(toml_str.contains("type = \"config\""));
    }

    #[test]
    fn output_manifest_minimal() {
        let manifest = OutputManifest {
            id: "minimal".into(),
            name: "Minimal".into(),
            version: "0.1.0".into(),
            description: None,
            author: None,
            entry_point: OutputEntryPoint::Wasm {
                path: "plugin.wasm".into(),
                hash: None,
            },
            capabilities: vec![],
            config: HashMap::new(),
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        assert!(toml_str.contains("id = \"minimal\""));
        assert!(!toml_str.contains("description"));
        assert!(!toml_str.contains("[[capabilities]]"));
    }

    #[test]
    fn generate_manifest_creates_file() {
        let dir = std::env::temp_dir().join("oc-bridge-test-output");
        let _ = std::fs::create_dir_all(&dir);

        // Create a fake WASM file
        let wasm_path = dir.join("plugin.wasm");
        let mut f = std::fs::File::create(&wasm_path).unwrap();
        f.write_all(b"fake wasm bytes").unwrap();

        let oc = OpenClawManifest {
            id: "test-plugin".into(),
            config_schema: serde_json::json!({}),
            name: Some("Test Plugin".into()),
            version: Some("1.0.0".into()),
            description: Some("A test".into()),
            kind: None,
            channels: vec![],
            providers: vec![],
            skills: vec![],
        };

        let config = HashMap::from([("key".into(), serde_json::json!("value"))]);

        generate_manifest("test-plugin", &oc, &wasm_path, &config, &dir).unwrap();

        let toml_path = dir.join("plugin.toml");
        assert!(toml_path.exists());

        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("id = \"test-plugin\""));
        assert!(content.contains("type = \"wasm\""));
        assert!(content.contains("hash = "));
        assert!(content.contains("key = \"value\""));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
