//! Plugin manifest types.
//!
//! A plugin manifest (`plugin.toml`) describes a plugin's identity, entry point,
//! required capabilities, and configuration. Manifests are loaded from disk
//! during plugin discovery.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use astrid_core::ConnectorProfile;
use astrid_core::identity::FrontendType;

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
    /// Connectors the plugin declares.
    ///
    /// These are manifest declarations only. Conversion to runtime
    /// [`ConnectorDescriptor`](astrid_core::ConnectorDescriptor) instances
    /// during plugin loading is not yet implemented — plugins currently
    /// expose connectors via [`Plugin::connectors()`](crate::Plugin::connectors).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connectors: Vec<ManifestConnector>,
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
    /// Plugin needs to register connectors.
    Connector {
        /// The behavioural profile of the connector.
        profile: ConnectorProfile,
    },
}

/// A connector declared in a plugin manifest.
///
/// This is a manifest declaration only — automatic conversion to a runtime
/// [`ConnectorDescriptor`](astrid_core::ConnectorDescriptor) during plugin
/// loading is not yet implemented.
///
/// Parsed from `[[connectors]]` sections in `plugin.toml`:
///
/// ```toml
/// [[connectors]]
/// name = "telegram"
/// platform = "telegram"   # parsed as FrontendType (e.g. "discord", "cli", or {"custom": "my-platform"})
/// profile = "chat"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestConnector {
    /// Human-readable connector name.
    pub name: String,
    /// The platform this connector serves.
    pub platform: FrontendType,
    /// Behavioural profile of the connector.
    pub profile: ConnectorProfile,
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
            author: Some("Astrid Team".into()),
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
            connectors: vec![],
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
            PluginCapability::Connector {
                profile: ConnectorProfile::Chat,
            },
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
        assert!(manifest.connectors.is_empty());
        assert!(manifest.config.is_empty());
    }

    #[test]
    fn test_manifest_with_connectors_toml() {
        let toml_str = r#"
            id = "telegram-bridge"
            name = "Telegram Bridge"
            version = "1.0.0"

            [entry_point]
            type = "mcp"
            command = "node"
            args = ["dist/index.js"]

            [[connectors]]
            name = "telegram"
            platform = "telegram"
            profile = "chat"

            [[connectors]]
            name = "cli-notify"
            platform = "cli"
            profile = "notify"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.connectors.len(), 2);
        assert_eq!(manifest.connectors[0].name, "telegram");
        assert_eq!(manifest.connectors[0].platform, FrontendType::Telegram);
        assert_eq!(manifest.connectors[0].profile, ConnectorProfile::Chat);
        assert_eq!(manifest.connectors[1].name, "cli-notify");
        assert_eq!(manifest.connectors[1].platform, FrontendType::Cli);
        assert_eq!(manifest.connectors[1].profile, ConnectorProfile::Notify);
    }

    #[test]
    fn test_manifest_without_connectors_backward_compat() {
        // Existing manifests without [[connectors]] must still parse.
        let toml_str = r#"
            id = "old-plugin"
            name = "Old Plugin"
            version = "0.1.0"

            [entry_point]
            type = "wasm"
            path = "plugin.wasm"

            [[capabilities]]
            type = "kv_store"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.id.as_str(), "old-plugin");
        assert!(manifest.connectors.is_empty());
        assert_eq!(manifest.capabilities.len(), 1);
    }

    #[test]
    fn test_connector_capability_serde() {
        let cap = PluginCapability::Connector {
            profile: ConnectorProfile::Chat,
        };
        let json = serde_json::to_string(&cap).unwrap();
        let parsed: PluginCapability = serde_json::from_str(&json).unwrap();
        match parsed {
            PluginCapability::Connector { profile } => {
                assert_eq!(profile, ConnectorProfile::Chat);
            },
            _ => panic!("expected Connector capability"),
        }
    }

    #[test]
    fn test_manifest_connector_serde_roundtrip() {
        let mc = ManifestConnector {
            name: "telegram".into(),
            platform: FrontendType::Telegram,
            profile: ConnectorProfile::Chat,
        };
        let json = serde_json::to_string(&mc).unwrap();
        let parsed: ManifestConnector = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "telegram");
        assert_eq!(parsed.platform, FrontendType::Telegram);
        assert_eq!(parsed.profile, ConnectorProfile::Chat);
    }

    #[test]
    fn test_connector_capability_toml_roundtrip() {
        let toml_str = r#"
            id = "connector-plugin"
            name = "Connector Plugin"
            version = "1.0.0"

            [entry_point]
            type = "wasm"
            path = "plugin.wasm"

            [[capabilities]]
            type = "connector"
            profile = "chat"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.capabilities.len(), 1);
        assert!(matches!(
            &manifest.capabilities[0],
            PluginCapability::Connector {
                profile: ConnectorProfile::Chat
            }
        ));
    }

    #[test]
    fn test_manifest_connector_custom_platform_toml() {
        let toml_str = r#"
            id = "custom-bridge"
            name = "Custom Bridge"
            version = "1.0.0"

            [entry_point]
            type = "wasm"
            path = "plugin.wasm"

            [[connectors]]
            name = "my-bridge"
            platform = { custom = "my-platform" }
            profile = "bridge"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.connectors.len(), 1);
        assert_eq!(
            manifest.connectors[0].platform,
            FrontendType::Custom("my-platform".into())
        );
        assert_eq!(manifest.connectors[0].profile, ConnectorProfile::Bridge);
    }
}
