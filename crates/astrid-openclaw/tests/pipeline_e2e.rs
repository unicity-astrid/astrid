//! End-to-end tests for the compilation pipeline.
//!
//! These tests exercise the full `compile_plugin` flow including tier detection,
//! caching, and output generation. WASM compilation tests are conditional on the
//! QuickJS kernel being available (skipped when using placeholder).

use std::collections::HashMap;
use std::path::Path;

use astrid_openclaw::pipeline::{CompileOptions, compile_plugin};
use astrid_openclaw::tier::PluginTier;

/// Create a realistic Tier 1 plugin with tools and config.
fn create_realistic_tier1_plugin(dir: &Path) {
    let manifest = r#"{
        "id": "search-tool",
        "name": "Web Search Tool",
        "version": "2.1.0",
        "description": "Search the web via API",
        "configSchema": {
            "type": "object",
            "properties": {
                "apiKey": {"type": "string", "description": "API key for search provider"},
                "maxResults": {"type": "number", "description": "Maximum results to return"}
            }
        }
    }"#;
    std::fs::write(dir.join("openclaw.plugin.json"), manifest).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();

    let source = r#"
module.exports.activate = function(ctx) {
    ctx.registerTool("search", {
        description: "Search the web",
        inputSchema: {
            type: "object",
            properties: {
                query: { type: "string", description: "Search query" }
            },
            required: ["query"]
        }
    }, async (args) => {
        const key = ctx.config.apiKey;
        return JSON.stringify({ results: ["result1", "result2"] });
    });
};
"#;
    std::fs::write(dir.join("src/index.js"), source).unwrap();
}

/// Create a realistic Tier 2 plugin with npm deps.
fn create_realistic_tier2_plugin(dir: &Path) {
    let manifest = r#"{
        "id": "slack-notifier",
        "name": "Slack Notifier",
        "version": "1.0.0",
        "description": "Send notifications to Slack",
        "configSchema": {
            "type": "object",
            "properties": {
                "slack_token": {"type": "string"},
                "channel": {"type": "string"}
            }
        }
    }"#;
    std::fs::write(dir.join("openclaw.plugin.json"), manifest).unwrap();

    let pkg = r#"{
        "name": "slack-notifier",
        "version": "1.0.0",
        "dependencies": {
            "@slack/web-api": "^6.0.0"
        }
    }"#;
    std::fs::write(dir.join("package.json"), pkg).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/index.js"),
        "const { WebClient } = require('@slack/web-api');\nmodule.exports = {};",
    )
    .unwrap();
}

#[test]
fn e2e_tier1_full_pipeline_js_only() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_realistic_tier1_plugin(plugin_dir.path());

    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    let result = compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: true,
        no_cache: true,
    })
    .unwrap();

    // Verify result metadata
    assert_eq!(result.astrid_id, "search-tool");
    assert_eq!(result.tier, PluginTier::Wasm);
    assert_eq!(result.manifest.display_name(), "Web Search Tool");
    assert_eq!(result.manifest.display_version(), "2.1.0");

    // Verify shim was generated
    let shim = std::fs::read_to_string(output_dir.path().join("shim.js")).unwrap();
    assert!(
        shim.contains("registerTool"),
        "shim should contain tool registration"
    );
    assert!(
        shim.contains("astrid_tool_call"),
        "shim should contain tool call export"
    );
}

#[test]
fn e2e_tier2_full_pipeline() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_realistic_tier2_plugin(plugin_dir.path());

    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    let result = compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: false,
        no_cache: true,
    })
    .unwrap();

    // Verify tier detection
    assert_eq!(result.tier, PluginTier::Node);
    assert_eq!(result.astrid_id, "slack-notifier");

    // Verify all output artifacts
    let capsule_toml = std::fs::read_to_string(output_dir.path().join("Capsule.toml")).unwrap();

    // Package metadata
    assert!(capsule_toml.contains(r#"name = "slack-notifier""#));
    assert!(capsule_toml.contains(r#"version = "1.0.0""#));

    // MCP server config
    assert!(capsule_toml.contains("[[mcp_server]]"));
    assert!(capsule_toml.contains(r#"command = "node""#));
    assert!(capsule_toml.contains("astrid_bridge.mjs"));

    // Capabilities
    assert!(capsule_toml.contains(r#"host_process = ["node"]"#));

    // Environment — slack_token should be detected as secret
    assert!(
        capsule_toml.contains("type = \"secret\""),
        "slack_token should be a secret, got:\n{capsule_toml}"
    );
    assert!(
        capsule_toml.contains("type = \"string\""),
        "channel should be a plain string"
    );

    // Bridge script
    let bridge = std::fs::read_to_string(output_dir.path().join("astrid_bridge.mjs")).unwrap();
    assert!(bridge.contains("handleInitialize"));

    // Source copied
    assert!(output_dir.path().join("src/src/index.js").exists());
}

#[test]
fn e2e_tier1_with_cache_hit() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_realistic_tier1_plugin(plugin_dir.path());

    let cache_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    // First compilation (js_only to avoid needing QuickJS kernel)
    let output1 = tempfile::tempdir().unwrap();
    let r1 = compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output1.path(),
        config: &config,
        cache_dir: Some(cache_dir.path()),
        js_only: true,
        no_cache: false,
    })
    .unwrap();

    // js_only doesn't populate cache (no WASM to cache), so both runs are fresh
    assert!(!r1.cached);

    // Second compilation with same source — should be idempotent
    let output2 = tempfile::tempdir().unwrap();
    let r2 = compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output2.path(),
        config: &config,
        cache_dir: Some(cache_dir.path()),
        js_only: true,
        no_cache: false,
    })
    .unwrap();

    assert_eq!(r1.astrid_id, r2.astrid_id);
    assert_eq!(r1.tier, r2.tier);

    // Both shims should be identical
    let shim1 = std::fs::read_to_string(output1.path().join("shim.js")).unwrap();
    let shim2 = std::fs::read_to_string(output2.path().join("shim.js")).unwrap();
    assert_eq!(
        shim1, shim2,
        "identical source should produce identical shims"
    );
}

#[test]
fn e2e_error_invalid_plugin_dir() {
    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    let result = compile_plugin(&CompileOptions {
        plugin_dir: Path::new("/nonexistent/plugin/dir"),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: false,
        no_cache: true,
    });

    assert!(result.is_err(), "should fail on nonexistent plugin dir");
}

#[test]
fn e2e_error_malformed_manifest() {
    let plugin_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        plugin_dir.path().join("openclaw.plugin.json"),
        "not valid json {{{",
    )
    .unwrap();

    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    let result = compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: false,
        no_cache: true,
    });

    assert!(result.is_err(), "should fail on malformed manifest");
}
