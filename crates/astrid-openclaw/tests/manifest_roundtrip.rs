//! Round-trip test: generated TOML parses correctly.
//!
//! Validates that the TOML output from `astrid-openclaw` contains the
//! expected fields and structure compatible with `CapsuleManifest`.

use std::collections::HashMap;

#[test]
fn generated_toml_parses_with_expected_fields() {
    let toml_str = r#"
        [package]
        name = "hello-tool"
        version = "1.0.0"
        description = "A test capsule"

        [component]
        entrypoint = "plugin.wasm"
        hash = "abc123def456"

        [env.api_key]
        type = "secret"
        request = "Please enter value for api_key"
    "#;

    let parsed: toml::Value = toml::from_str(toml_str).expect("should parse as valid TOML");
    let table = parsed.as_table().unwrap();

    let package = table["package"].as_table().unwrap();
    assert_eq!(package["name"].as_str().unwrap(), "hello-tool");
    assert_eq!(package["version"].as_str().unwrap(), "1.0.0");
    assert_eq!(package["description"].as_str().unwrap(), "A test capsule");

    let component = table["component"].as_table().unwrap();
    assert_eq!(component["entrypoint"].as_str().unwrap(), "plugin.wasm");
    assert_eq!(component["hash"].as_str().unwrap(), "abc123def456");

    let env = table["env"].as_table().unwrap();
    let api_key = env["api_key"].as_table().unwrap();
    assert_eq!(api_key["type"].as_str().unwrap(), "secret");
}

#[test]
fn minimal_generated_toml_parses() {
    let toml_str = r#"
        [package]
        name = "minimal-plugin"
        version = "0.1.0"

        [component]
        entrypoint = "plugin.wasm"
    "#;

    let parsed: toml::Value = toml::from_str(toml_str).expect("should parse as valid TOML");
    let table = parsed.as_table().unwrap();

    let package = table["package"].as_table().unwrap();
    assert_eq!(package["name"].as_str().unwrap(), "minimal-plugin");
    assert!(table.get("env").is_none());
}

#[test]
fn output_manifest_round_trips_through_toml() {
    let mut config = HashMap::new();
    config.insert("debug".to_string(), serde_json::json!(true));

    let oc = astrid_openclaw::manifest::OpenClawManifest {
        id: "my-cool-plugin".into(),
        name: Some("My Cool Plugin".into()),
        version: Some("2.0.0".into()),
        description: Some("Cool stuff".into()),
        config_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "apiKey": { "type": "string" }
            }
        }),
        kind: None,
        channels: vec![],
        providers: vec![],
        skills: vec![],
    };

    let astrid_id = astrid_openclaw::manifest::convert_id(&oc.id).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let wasm_path = dir.path().join("plugin.wasm");
    std::fs::write(&wasm_path, b"fake wasm content").unwrap();

    astrid_openclaw::output::generate_manifest(&astrid_id, &oc, &wasm_path, &config, dir.path())
        .unwrap();

    let toml_content = std::fs::read_to_string(dir.path().join("Capsule.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&toml_content).expect("generated TOML should parse");
    let table = parsed.as_table().unwrap();

    let package = table["package"].as_table().unwrap();
    assert_eq!(package["name"].as_str().unwrap(), "my-cool-plugin");
    assert_eq!(package["version"].as_str().unwrap(), "2.0.0");
    assert_eq!(package["description"].as_str().unwrap(), "Cool stuff");

    let component = table["component"].as_table().unwrap();
    assert_eq!(component["entrypoint"].as_str().unwrap(), "plugin.wasm");
    assert!(component.contains_key("hash"));

    let env = table["env"].as_table().unwrap();
    let api_key = env["apiKey"].as_table().unwrap();
    assert_eq!(api_key["type"].as_str().unwrap(), "secret");
}
