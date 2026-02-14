//! Round-trip test: generated TOML parses as `astralis_plugins::manifest::PluginManifest`.
//!
//! This test uses `astralis-plugins` as a dev-dependency to verify that the
//! TOML output from `openclaw-bridge` is serde-compatible with the real
//! `PluginManifest` type.

use std::collections::HashMap;

use astralis_plugins::manifest::PluginManifest;

#[test]
fn generated_toml_parses_as_plugin_manifest() {
    // This TOML mirrors what output::generate_manifest produces.
    let toml_str = r#"
        id = "hello-tool"
        name = "Hello Tool"
        version = "1.0.0"
        description = "A test plugin"

        [entry_point]
        type = "wasm"
        path = "plugin.wasm"
        hash = "abc123def456"

        [[capabilities]]
        type = "config"

        [config]
        timeout = 30
        api_key = "test"
    "#;

    let manifest: PluginManifest =
        toml::from_str(toml_str).expect("should parse as PluginManifest");

    assert_eq!(manifest.id.as_str(), "hello-tool");
    assert_eq!(manifest.name, "Hello Tool");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.description.as_deref(), Some("A test plugin"));
    assert_eq!(manifest.capabilities.len(), 1);
    assert_eq!(manifest.config.len(), 2);
}

#[test]
fn minimal_generated_toml_parses() {
    let toml_str = r#"
        id = "minimal-plugin"
        name = "Minimal"
        version = "0.1.0"

        [entry_point]
        type = "wasm"
        path = "plugin.wasm"
    "#;

    let manifest: PluginManifest =
        toml::from_str(toml_str).expect("should parse as PluginManifest");
    assert_eq!(manifest.id.as_str(), "minimal-plugin");
    assert!(manifest.capabilities.is_empty());
    assert!(manifest.config.is_empty());
}

#[test]
fn output_manifest_round_trips_through_real_type() {
    // Build a manifest using openclaw-bridge's output types, serialize to TOML,
    // then parse with the real PluginManifest type.
    let mut config = HashMap::new();
    config.insert("debug".to_string(), serde_json::json!(true));

    let oc = openclaw_bridge::manifest::OpenClawManifest {
        id: "my-cool-plugin".into(),
        name: "My Cool Plugin".into(),
        version: "2.0.0".into(),
        description: Some("Cool stuff".into()),
        main: "index.js".into(),
        engines: None,
    };

    // Use the same serialization logic as output.rs
    let astralis_id = openclaw_bridge::manifest::convert_id(&oc.id).unwrap();

    // Create a temp dir with a fake WASM file
    let dir = std::env::temp_dir().join("oc-bridge-roundtrip-test");
    let _ = std::fs::create_dir_all(&dir);
    let wasm_path = dir.join("plugin.wasm");
    std::fs::write(&wasm_path, b"fake wasm content").unwrap();

    openclaw_bridge::output::generate_manifest(&astralis_id, &oc, &wasm_path, &config, &dir)
        .unwrap();

    let toml_content = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    let parsed: PluginManifest =
        toml::from_str(&toml_content).expect("generated TOML should parse as PluginManifest");

    assert_eq!(parsed.id.as_str(), "my-cool-plugin");
    assert_eq!(parsed.name, "My Cool Plugin");
    assert_eq!(parsed.version, "2.0.0");
    assert_eq!(parsed.config.get("debug"), Some(&serde_json::json!(true)));

    let _ = std::fs::remove_dir_all(&dir);
}
