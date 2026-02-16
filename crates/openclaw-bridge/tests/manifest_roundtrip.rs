//! Round-trip test: generated TOML parses correctly.
//!
//! Validates that the TOML output from `openclaw-bridge` contains the
//! expected fields and structure compatible with `PluginManifest`.
//! We parse as `toml::Value` to avoid a dev-dependency on `astrid-plugins`,
//! which would pull in `extism`/`wasmtime` and conflict with wizer's wasmtime.

use std::collections::HashMap;

#[test]
fn generated_toml_parses_with_expected_fields() {
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

    let parsed: toml::Value = toml::from_str(toml_str).expect("should parse as valid TOML");
    let table = parsed.as_table().unwrap();

    assert_eq!(table["id"].as_str().unwrap(), "hello-tool");
    assert_eq!(table["name"].as_str().unwrap(), "Hello Tool");
    assert_eq!(table["version"].as_str().unwrap(), "1.0.0");
    assert_eq!(table["description"].as_str().unwrap(), "A test plugin");

    let entry = table["entry_point"].as_table().unwrap();
    assert_eq!(entry["type"].as_str().unwrap(), "wasm");
    assert_eq!(entry["path"].as_str().unwrap(), "plugin.wasm");

    let caps = table["capabilities"].as_array().unwrap();
    assert_eq!(caps.len(), 1);

    let config = table["config"].as_table().unwrap();
    assert_eq!(config.len(), 2);
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

    let parsed: toml::Value = toml::from_str(toml_str).expect("should parse as valid TOML");
    let table = parsed.as_table().unwrap();

    assert_eq!(table["id"].as_str().unwrap(), "minimal-plugin");
    assert!(table.get("capabilities").is_none());
    assert!(table.get("config").is_none());
}

#[test]
fn output_manifest_round_trips_through_toml() {
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

    let astrid_id = openclaw_bridge::manifest::convert_id(&oc.id).unwrap();

    let dir = std::env::temp_dir().join("oc-bridge-roundtrip-test");
    let _ = std::fs::create_dir_all(&dir);
    let wasm_path = dir.join("plugin.wasm");
    std::fs::write(&wasm_path, b"fake wasm content").unwrap();

    openclaw_bridge::output::generate_manifest(&astrid_id, &oc, &wasm_path, &config, &dir).unwrap();

    let toml_content = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&toml_content).expect("generated TOML should parse");
    let table = parsed.as_table().unwrap();

    assert_eq!(table["id"].as_str().unwrap(), "my-cool-plugin");
    assert_eq!(table["name"].as_str().unwrap(), "My Cool Plugin");
    assert_eq!(table["version"].as_str().unwrap(), "2.0.0");

    let config_table = table["config"].as_table().unwrap();
    assert_eq!(config_table["debug"].as_bool().unwrap(), true);

    let _ = std::fs::remove_dir_all(&dir);
}
