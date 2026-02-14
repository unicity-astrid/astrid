//! Plugin manifest types.
//!
//! A plugin manifest (`plugin.toml`) describes a plugin's identity, entry point,
//! required capabilities, and configuration. Manifests are loaded from disk
//! during plugin discovery.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::PluginId;

/// A plugin manifest loaded from `plugin.toml`.
///
/// Describes everything the runtime needs to know about a plugin before
/// loading it: identity, entry point, capability requirements, and config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique plugin identifier.
    pub id: PluginId,
    /// Human-readable display name.
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// How to run this plugin.
    pub entry_point: PluginEntryPoint,
    /// Capabilities the plugin requires from the runtime.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<PluginCapability>,
    /// Arbitrary plugin configuration.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub config: HashMap<String, serde_json::Value>,
}

/// How a plugin is executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginEntryPoint {
    /// A WASM module loaded via Extism/Wasmtime.
    Wasm {
        /// Path to the `.wasm` file (relative to the plugin directory).
        path: PathBuf,
        /// Optional blake3 hex digest for integrity verification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hash: Option<String>,
    },
    /// An MCP server spawned as a child process.
    Mcp {
        /// Command to run (e.g. `"npx"`, `"python"`).
        command: String,
        /// Arguments (e.g. `["-y", "@modelcontextprotocol/server-filesystem"]`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        /// Additional environment variables.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        env: HashMap<String, String>,
        /// Expected binary hash (`sha256:...`) for verification before launch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binary_hash: Option<String>,
    },
}

/// A capability that a plugin requires from the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginCapability {
    /// Plugin needs HTTP access to specific hosts.
    HttpAccess {
        /// Allowed host patterns (e.g. `["api.github.com", "*.example.com"]`).
        hosts: Vec<String>,
    },
    /// Plugin needs to read files at specific paths.
    FileRead {
        /// Allowed path patterns (e.g. `["./src/**", "/tmp/**"]`).
        paths: Vec<String>,
    },
    /// Plugin needs to write files at specific paths.
    FileWrite {
        /// Allowed path patterns.
        paths: Vec<String>,
    },
    /// Plugin needs access to scoped key-value storage.
    KvStore,
    /// Plugin needs access to its configuration.
    Config,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            id: PluginId::from_static("test-plugin"),
            name: "Test Plugin".into(),
            version: "0.1.0".into(),
            description: Some("A test plugin".into()),
            author: Some("Astralis Team".into()),
            entry_point: PluginEntryPoint::Wasm {
                path: PathBuf::from("plugin.wasm"),
                hash: None,
            },
            capabilities: vec![
                PluginCapability::KvStore,
                PluginCapability::HttpAccess {
                    hosts: vec!["api.github.com".into()],
                },
            ],
            config: HashMap::from([("timeout".into(), serde_json::json!(30))]),
        }
    }

    #[test]
    fn test_manifest_toml_round_trip() {
        let manifest = sample_manifest();
        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        let parsed: PluginManifest = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.id, manifest.id);
        assert_eq!(parsed.name, manifest.name);
        assert_eq!(parsed.version, manifest.version);
        assert_eq!(parsed.description, manifest.description);
        assert_eq!(parsed.author, manifest.author);
        assert_eq!(parsed.capabilities.len(), manifest.capabilities.len());
    }

    #[test]
    fn test_manifest_json_round_trip() {
        let manifest = sample_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: PluginManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, manifest.id);
        assert_eq!(parsed.name, manifest.name);
    }

    #[test]
    fn test_wasm_entry_point_serde() {
        let ep = PluginEntryPoint::Wasm {
            path: PathBuf::from("plugin.wasm"),
            hash: None,
        };
        let toml_str = toml::to_string(&ep).unwrap();
        assert!(toml_str.contains("type = \"wasm\""));
    }

    #[test]
    fn test_mcp_entry_point_serde() {
        let ep = PluginEntryPoint::Mcp {
            command: "npx".into(),
            args: vec!["-y".into(), "@mcp/server-fs".into()],
            env: HashMap::from([("NODE_ENV".into(), "production".into())]),
            binary_hash: None,
        };
        let toml_str = toml::to_string(&ep).unwrap();
        assert!(toml_str.contains("type = \"mcp\""));
        assert!(toml_str.contains("npx"));
    }

    #[test]
    fn test_capability_variants_serde() {
        let caps = vec![
            PluginCapability::HttpAccess {
                hosts: vec!["*.example.com".into()],
            },
            PluginCapability::FileRead {
                paths: vec!["./src/**".into()],
            },
            PluginCapability::FileWrite {
                paths: vec!["/tmp/**".into()],
            },
            PluginCapability::KvStore,
            PluginCapability::Config,
        ];

        for cap in &caps {
            let json = serde_json::to_string(cap).unwrap();
            let _parsed: PluginCapability = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn test_minimal_manifest_toml() {
        let toml_str = r#"
            id = "minimal-plugin"
            name = "Minimal Plugin"
            version = "0.1.0"

            [entry_point]
            type = "wasm"
            path = "plugin.wasm"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.id.as_str(), "minimal-plugin");
        assert!(manifest.description.is_none());
        assert!(manifest.capabilities.is_empty());
        assert!(manifest.config.is_empty());
    }
}
