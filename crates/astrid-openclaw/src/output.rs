//! Astrid plugin manifest (`plugin.toml`) generation and output assembly.
//!
//! Generates a `Capsule.toml` that is serde-compatible with
//! the capsule manifest format, including the tagged enum
//! format for `entry_point` and `capabilities`.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{BridgeError, BridgeResult};
use crate::manifest::OpenClawManifest;

/// Fallback version when the `OpenClaw` manifest has no version or an invalid one.
const DEFAULT_VERSION: &str = "0.0.0";

/// Sanitize an `OpenClaw` version string into valid semver.
///
/// Returns the version as-is if it's valid semver, otherwise falls back to
/// `"0.0.0"` with a warning. This ensures the generated `Capsule.toml` always
/// contains a valid semver version, matching Cargo.toml conventions.
fn sanitize_version(raw: &str) -> String {
    if semver::Version::parse(raw).is_ok() {
        raw.to_string()
    } else {
        eprintln!("warning: OpenClaw version '{raw}' is not valid semver, using {DEFAULT_VERSION}");
        DEFAULT_VERSION.to_string()
    }
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keywords: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    enum_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
}

/// Generate `Capsule.toml` in the output directory.
///
/// Reads the compiled WASM file to compute a blake3 hash, then writes
/// the manifest with the correct `CapsuleManifest` serde format.
///
/// # Errors
///
/// Returns `BridgeError::Output` if the WASM file cannot be read or the TOML cannot be written.
#[expect(clippy::implicit_hasher)]
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
    for (key, f) in crate::manifest::extract_env_fields(oc_manifest)? {
        env.insert(
            key,
            EnvDef {
                env_type: f.env_type,
                request: Some(f.request),
                description: f.description,
                default: f.default,
                enum_values: f.enum_values,
                placeholder: f.placeholder,
            },
        );
    }

    // Map OpenClaw `kind` → Astrid `categories`, `skills` → `keywords`
    let categories = oc_manifest
        .kind
        .as_deref()
        .map(|k| vec![k.to_string()])
        .unwrap_or_default();
    let keywords = oc_manifest.skills.clone();

    let manifest = OutputManifest {
        package: PackageDef {
            name: astrid_id.to_string(),
            version: sanitize_version(oc_manifest.display_version()),
            description: oc_manifest.description.clone(),
            categories,
            keywords,
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
                categories: vec!["tool".into()],
                keywords: vec!["code-assist".into()],
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
                    description: None,
                    default: None,
                    enum_values: vec![],
                    placeholder: None,
                },
            )]),
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        assert!(toml_str.contains("name = \"hello-tool\""));
        assert!(toml_str.contains("entrypoint = \"plugin.wasm\""));
        assert!(toml_str.contains("hash = \"abc123\""));
        assert!(toml_str.contains("type = \"secret\""));
        assert!(
            toml_str.contains("categories = [\"tool\"]"),
            "kind should map to categories"
        );
        assert!(
            toml_str.contains("keywords = [\"code-assist\"]"),
            "skills should map to keywords"
        );
    }

    #[test]
    fn output_manifest_array_field_type() {
        let manifest = OutputManifest {
            package: PackageDef {
                name: "relay-plugin".into(),
                version: "1.0.0".into(),
                description: None,
                categories: vec![],
                keywords: vec![],
            },
            component: Some(ComponentDef {
                entrypoint: "plugin.wasm".into(),
                hash: None,
            }),
            env: HashMap::from([(
                "additionalRelays".into(),
                EnvDef {
                    env_type: "array".into(),
                    request: Some("Enter relay URLs".into()),
                    description: None,
                    default: None,
                    enum_values: vec![],
                    placeholder: None,
                },
            )]),
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        assert!(
            toml_str.contains("type = \"array\""),
            "array fields should serialize with type = array"
        );
    }

    #[test]
    fn output_manifest_minimal() {
        let manifest = OutputManifest {
            package: PackageDef {
                name: "minimal".into(),
                version: "0.1.0".into(),
                description: None,
                categories: vec![],
                keywords: vec![],
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
            kind: Some("tool".into()),
            channels: vec![],
            providers: vec![],
            skills: vec!["code-review".into()],
            ui_hints: serde_json::Value::Null,
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
        assert!(
            content.contains("categories"),
            "kind should be mapped to categories"
        );
        assert!(
            content.contains("code-review"),
            "skills should be mapped to keywords"
        );
    }

    #[test]
    fn generate_manifest_preserves_enum_default_description() {
        let dir = tempfile::tempdir().unwrap();

        let wasm_path = dir.path().join("plugin.wasm");
        let mut f = std::fs::File::create(&wasm_path).unwrap();
        f.write_all(b"fake wasm bytes").unwrap();

        let oc = OpenClawManifest {
            id: "unicity-plugin".into(),
            config_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "network": {
                        "type": "string",
                        "enum": ["testnet", "mainnet", "dev"],
                        "default": "testnet",
                        "description": "Target network"
                    },
                    "owner": {
                        "type": "string",
                        "description": "Wallet address"
                    }
                }
            }),
            name: Some("Unicity Plugin".into()),
            version: Some("1.0.0".into()),
            description: Some("Test".into()),
            kind: None,
            channels: vec![],
            providers: vec![],
            skills: vec![],
            ui_hints: serde_json::Value::Null,
        };

        let config = HashMap::new();
        generate_manifest("unicity-plugin", &oc, &wasm_path, &config, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("Capsule.toml")).unwrap();

        // network should have enum_values, default, and description
        assert!(
            content.contains("enum_values"),
            "enum_values should be emitted: {content}"
        );
        assert!(
            content.contains("testnet"),
            "enum choice should be present: {content}"
        );
        assert!(
            content.contains("default = \"testnet\""),
            "default should be preserved: {content}"
        );
        assert!(
            content.contains("description = \"Target network\""),
            "description should be preserved: {content}"
        );

        // owner should have description but no enum_values
        assert!(
            content.contains("description = \"Wallet address\""),
            "owner description should be preserved: {content}"
        );
    }

    #[test]
    fn generate_manifest_handles_missing_schema_fields() {
        let dir = tempfile::tempdir().unwrap();

        let wasm_path = dir.path().join("plugin.wasm");
        let mut f = std::fs::File::create(&wasm_path).unwrap();
        f.write_all(b"fake wasm bytes").unwrap();

        let oc = OpenClawManifest {
            id: "simple-plugin".into(),
            config_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "plainField": { "type": "string" }
                }
            }),
            name: None,
            version: None,
            description: None,
            kind: None,
            channels: vec![],
            providers: vec![],
            skills: vec![],
            ui_hints: serde_json::Value::Null,
        };

        let config = HashMap::new();
        generate_manifest("simple-plugin", &oc, &wasm_path, &config, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("Capsule.toml")).unwrap();

        // No enum_values, no default, no description for plainField
        assert!(
            !content.contains("enum_values"),
            "should skip empty enum_values: {content}"
        );
        assert!(
            !content.contains("description"),
            "should skip empty description: {content}"
        );
    }
}
