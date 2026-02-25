//! Astrid plugin manifest (`plugin.toml`) generation and output assembly.
//!
//! Generates a `plugin.toml` that is serde-compatible with
//! [`astrid_plugins::manifest::PluginManifest`] including the tagged enum
//! format for `entry_point` and `capabilities`.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{BridgeError, BridgeResult};
use crate::manifest::OpenClawManifest;

/// A serializable Astrid capsule manifest matching the `CapsuleManifest` serde format.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OutputManifest {
    package: PackageDef,
    component: Option<ComponentDef>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, EnvDef>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PackageDef {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ComponentDef {
    entrypoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct EnvDef {
    #[serde(rename = "type")]
    env_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<String>,
}

/// Generate `Capsule.toml` in the output directory.
///
/// Reads the compiled WASM file to compute a blake3 hash, then writes
/// the manifest with the correct `CapsuleManifest` serde format.
///
/// # Errors
///
/// Returns `BridgeError::Output` if the WASM file cannot be read or the TOML cannot be written.
#[allow(clippy::implicit_hasher)]
pub fn generate_manifest(
    astrid_id: &str,
    oc_manifest: &OpenClawManifest,
    wasm_path: &Path,
    _config: &HashMap<String, serde_json::Value>,
    output_dir: &Path,
) -> BridgeResult<()> {
    // Compute blake3 hash of the WASM file
    let wasm_bytes = std::fs::read(wasm_path)
        .map_err(|e| BridgeError::Output(format!("failed to read WASM file for hashing: {e}")))?;
    let hash = blake3::hash(&wasm_bytes).to_hex().to_string();

    let mut env = HashMap::new();
    // Map configSchema to env elicitations
    if let Some(obj) = oc_manifest.config_schema.as_object()
        && let Some(props) = obj.get("properties").and_then(|p| p.as_object())
    {
        for (key, _val) in props {
            let lower = key.to_lowercase();
            let is_secret = lower.contains("api_key")
                || lower.contains("apikey")
                || lower == "token"
                || lower.ends_with("_token")
                || lower == "secret"
                || lower == "password";
            env.insert(
                key.clone(),
                EnvDef {
                    env_type: if is_secret { "secret" } else { "string" }.into(),
                    request: Some(format!("Please enter value for {key}")),
                },
            );
        }
    }

    let manifest = OutputManifest {
        package: PackageDef {
            name: astrid_id.to_string(),
            version: oc_manifest.display_version().to_string(),
            description: oc_manifest.description.clone(),
        },
        component: Some(ComponentDef {
            entrypoint: "plugin.wasm".into(),
            hash: Some(hash),
        }),
        env,
    };

    let toml_str = toml::to_string_pretty(&manifest)
        .map_err(|e| BridgeError::Output(format!("failed to serialize Capsule.toml: {e}")))?;

    let toml_path = output_dir.join("Capsule.toml");
    std::fs::write(&toml_path, toml_str)
        .map_err(|e| BridgeError::Output(format!("failed to write Capsule.toml: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn output_manifest_serializes_correctly() {
        let manifest = OutputManifest {
            package: PackageDef {
                name: "hello-tool".into(),
                version: "1.0.0".into(),
                description: Some("A test plugin".into()),
            },
            component: Some(ComponentDef {
                entrypoint: "plugin.wasm".into(),
                hash: Some("abc123".into()),
            }),
            env: HashMap::from([(
                "apiKey".into(),
                EnvDef {
                    env_type: "secret".into(),
                    request: Some("Enter key".into()),
                },
            )]),
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        assert!(toml_str.contains("name = \"hello-tool\""));
        assert!(toml_str.contains("entrypoint = \"plugin.wasm\""));
        assert!(toml_str.contains("hash = \"abc123\""));
        assert!(toml_str.contains("type = \"secret\""));
    }

    #[test]
    fn output_manifest_minimal() {
        let manifest = OutputManifest {
            package: PackageDef {
                name: "minimal".into(),
                version: "0.1.0".into(),
                description: None,
            },
            component: Some(ComponentDef {
                entrypoint: "plugin.wasm".into(),
                hash: None,
            }),
            env: HashMap::new(),
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        assert!(toml_str.contains("name = \"minimal\""));
        assert!(!toml_str.contains("description"));
        assert!(!toml_str.contains("[env]"));
    }

    #[test]
    fn generate_manifest_creates_file() {
        let dir = tempfile::tempdir().unwrap();

        // Create a fake WASM file
        let wasm_path = dir.path().join("plugin.wasm");
        let mut f = std::fs::File::create(&wasm_path).unwrap();
        f.write_all(b"fake wasm bytes").unwrap();

        let oc = OpenClawManifest {
            id: "test-plugin".into(),
            config_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "apiKey": { "type": "string" }
                }
            }),
            name: Some("Test Plugin".into()),
            version: Some("1.0.0".into()),
            description: Some("A test".into()),
            kind: None,
            channels: vec![],
            providers: vec![],
            skills: vec![],
        };

        let config = HashMap::new();

        generate_manifest("test-plugin", &oc, &wasm_path, &config, dir.path()).unwrap();

        let toml_path = dir.path().join("Capsule.toml");
        assert!(toml_path.exists());

        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("name = \"test-plugin\""));
        assert!(content.contains("entrypoint = \"plugin.wasm\""));
        assert!(content.contains("hash = "));
        assert!(content.contains("type = \"secret\""));
    }
}
