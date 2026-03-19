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

    // MCP server config — must pass --entry and --plugin-id flags
    assert!(capsule_toml.contains("[[mcp_server]]"));
    assert!(
        capsule_toml.contains(r#"command = "node""#) || capsule_toml.contains(r#"command = "/"#),
        "Tier 2 should use node"
    );
    assert!(capsule_toml.contains("astrid_bridge.mjs"));
    assert!(
        capsule_toml.contains("--entry"),
        "args must include --entry flag for the bridge script, got:\n{capsule_toml}"
    );
    assert!(
        capsule_toml.contains("--plugin-id"),
        "args must include --plugin-id flag for the bridge script, got:\n{capsule_toml}"
    );

    // Capabilities
    assert!(
        capsule_toml.contains(r#"host_process = ["node"]"#)
            || capsule_toml.contains(r#"host_process = ["/"#),
        "Tier 2 should declare host_process"
    );

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
    assert!(output_dir.path().join("src/index.js").exists());
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

/// Create a plugin following official OpenClaw conventions exactly:
/// - Object export with `register()` method (not `activate()`)
/// - `package.json` with `openclaw.extensions: ["./src/index.js"]` (dotslash prefix)
/// - `registerTool` with `parameters` (not `inputSchema`)
/// - `registerCommand` with full options object
/// - `registerHook` with 3rd metadata arg
/// - `on()` with priority option
/// - `configSchema` with `required` and `uiHints`
fn create_official_openclaw_plugin(dir: &Path) {
    let manifest = r#"{
        "id": "greeting-tool",
        "name": "Greeting Tool",
        "version": "0.1.0",
        "description": "A greeting plugin following official OpenClaw conventions",
        "kind": "tool",
        "configSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "defaultGreeting": {
                    "type": "string",
                    "description": "The default greeting to use",
                    "default": "Hello"
                },
                "apiKey": {
                    "type": "string",
                    "description": "API key for premium greetings"
                }
            },
            "required": ["defaultGreeting"]
        },
        "uiHints": {
            "apiKey": {
                "label": "API Key",
                "sensitive": true,
                "placeholder": "sk-..."
            }
        }
    }"#;
    std::fs::write(dir.join("openclaw.plugin.json"), manifest).unwrap();

    // package.json with ./src/index.js entry (real plugins use ./ prefix)
    let pkg = r#"{
        "name": "openclaw-plugin-greeting",
        "version": "0.1.0",
        "type": "module",
        "main": "src/index.js",
        "openclaw": {
            "extensions": ["./src/index.js"]
        },
        "peerDependencies": {
            "openclaw": ">=2025.1.0"
        }
    }"#;
    std::fs::write(dir.join("package.json"), pkg).unwrap();

    std::fs::create_dir_all(dir.join("src")).unwrap();

    // Plugin source using official conventions
    let source = r#"
export default {
    id: "greeting-tool",
    name: "Greeting Tool",
    description: "A simple greeting plugin",
    kind: "tool",

    register(api) {
        const config = api.config || {};
        const defaultGreeting = config.defaultGreeting || "Hello";

        api.registerTool({
            name: "send_greeting",
            description: "Send a personalized greeting",
            parameters: {
                type: "object",
                properties: {
                    name: { type: "string", description: "Name to greet" },
                    style: { type: "string", enum: ["formal", "casual"] }
                },
                required: ["name"]
            },
            execute: async (_id, params) => {
                return { content: [{ type: "text", text: defaultGreeting + ", " + params.name }] };
            }
        });

        api.registerCommand({
            name: "greet",
            description: "Quick greeting command",
            acceptsArgs: true,
            handler: async (ctx) => {
                return { text: defaultGreeting + ", " + (ctx.args.join(" ") || "world") };
            }
        });

        api.registerHook("message:received", async (event) => {
            return null;
        }, {
            name: "greeting-tool.detector",
            description: "Detect friendly messages"
        });

        api.on("prompt_builder.v1.hook.before_build", (event, ctx) => {
            return { appendSystemContext: "Be friendly." };
        }, { priority: 50 });

        api.logger.info("Greeting Tool registered");
    }
};
"#;
    std::fs::write(dir.join("src/index.js"), source).unwrap();
}

/// Test: A real OpenClaw plugin following official conventions compiles through Tier 1.
#[test]
fn e2e_official_openclaw_plugin_tier1() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_official_openclaw_plugin(plugin_dir.path());

    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    let result = compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: true, // Skip WASM compilation (kernel may not be built)
        no_cache: true,
    })
    .expect("official OpenClaw plugin should compile successfully");

    // Verify metadata
    assert_eq!(result.astrid_id, "greeting-tool");
    assert_eq!(result.tier, PluginTier::Wasm, "no npm deps → Tier 1");
    assert_eq!(result.manifest.display_name(), "Greeting Tool");
    assert_eq!(result.manifest.display_version(), "0.1.0");

    // Verify shim was generated
    let shim = std::fs::read_to_string(output_dir.path().join("shim.js")).unwrap();

    // Must contain all registration APIs
    assert!(
        shim.contains("registerTool"),
        "shim must support registerTool"
    );
    assert!(
        shim.contains("registerCommand"),
        "shim must support registerCommand"
    );
    assert!(
        shim.contains("registerHook"),
        "shim must support registerHook"
    );

    // Must support register() pattern (not just activate)
    assert!(
        shim.contains("register") && shim.contains("activate"),
        "shim must support both register() and activate() patterns"
    );

    // Must contain all WASM exports
    assert!(
        shim.contains("astrid_tool_call"),
        "shim must export astrid_tool_call"
    );
    assert!(
        shim.contains("astrid_hook_trigger"),
        "shim must export astrid_hook_trigger"
    );
    assert!(
        shim.contains("describe-tools"),
        "shim must export describe-tools"
    );

    // Plugin source should be embedded in the shim
    assert!(
        shim.contains("send_greeting") || shim.contains("greeting"),
        "shim should contain the plugin source"
    );

    // Capsule.toml must always be generated — every capsule needs a manifest
    let capsule_toml = output_dir.path().join("Capsule.toml");
    assert!(capsule_toml.exists(), "Capsule.toml must be generated");
    let toml_content = std::fs::read_to_string(&capsule_toml).unwrap();
    assert!(toml_content.contains(r#"name = "greeting-tool""#));

    // apiKey should be detected as a secret in env section
    assert!(
        toml_content.contains("type = \"secret\""),
        "apiKey should be detected as secret, got:\n{toml_content}"
    );
}

/// Test: An official OpenClaw plugin with npm deps compiles through Tier 2.
#[test]
fn e2e_official_openclaw_plugin_tier2() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_official_openclaw_plugin(plugin_dir.path());

    // Add npm dependencies to force Tier 2
    let pkg = r#"{
        "name": "openclaw-plugin-greeting",
        "version": "0.1.0",
        "type": "module",
        "main": "src/index.js",
        "dependencies": {
            "got": "^14.0.0"
        },
        "openclaw": {
            "extensions": ["./src/index.js"]
        }
    }"#;
    std::fs::write(plugin_dir.path().join("package.json"), pkg).unwrap();

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
    .expect("official OpenClaw plugin with deps should compile as Tier 2");

    // Verify Tier 2 detection
    assert_eq!(result.tier, PluginTier::Node, "npm deps → Tier 2");
    assert_eq!(result.astrid_id, "greeting-tool");

    // Verify Capsule.toml has MCP server config
    let capsule_toml = std::fs::read_to_string(output_dir.path().join("Capsule.toml")).unwrap();
    assert!(capsule_toml.contains("[[mcp_server]]"));
    assert!(
        capsule_toml.contains(r#"command = "node""#) || capsule_toml.contains(r#"command = "/"#),
        "Tier 2 should use node"
    );
    assert!(capsule_toml.contains("--entry"));
    assert!(
        capsule_toml.contains("src/index.js"),
        "entry point should be src/index.js (stripped ./), got:\n{capsule_toml}"
    );

    // Bridge script must be present
    assert!(output_dir.path().join("astrid_bridge.mjs").exists());

    // Source must be copied
    assert!(output_dir.path().join("src/index.js").exists());

    // package.json must be copied (needed for module resolution)
    assert!(output_dir.path().join("package.json").exists());
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

/// Helper: create a Tier 2 channel plugin with configurable channels and uiHints.
fn create_channel_plugin(dir: &Path, channels: &[&str], ui_hints: serde_json::Value) {
    let manifest = serde_json::json!({
        "id": "channel-plugin",
        "name": "Channel Plugin",
        "version": "1.0.0",
        "description": "A channel plugin",
        "configSchema": {
            "type": "object",
            "properties": {
                "myCredential": { "type": "string" },
                "network": {
                    "type": "string",
                    "enum": ["testnet", "mainnet"],
                    "default": "testnet"
                }
            }
        },
        "channels": channels,
        "uiHints": ui_hints,
    });
    std::fs::write(
        dir.join("openclaw.plugin.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let pkg = r#"{"name": "channel-plugin", "dependencies": {"ws": "^8.0.0"}}"#;
    std::fs::write(dir.join("package.json"), pkg).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/index.js"),
        "const ws = require('ws');\nmodule.exports = {};",
    )
    .unwrap();
}

#[test]
fn tier2_channel_plugin_generates_uplink_and_capability() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_channel_plugin(plugin_dir.path(), &["unicity"], serde_json::json!({}));

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

    assert_eq!(result.tier, PluginTier::Node);

    let capsule_toml = std::fs::read_to_string(output_dir.path().join("Capsule.toml")).unwrap();

    // Parse as TOML value to check structure
    let parsed: toml::Value = toml::from_str(&capsule_toml).unwrap();

    // Check [[uplink]] array
    let uplinks = parsed.get("uplink").unwrap().as_array().unwrap();
    assert_eq!(uplinks.len(), 1);
    let uplink = &uplinks[0];
    assert_eq!(uplink.get("name").unwrap().as_str().unwrap(), "unicity");
    assert_eq!(uplink.get("profile").unwrap().as_str().unwrap(), "bridge");
    // Platform is the lowercased channel name
    assert_eq!(
        uplink.get("platform").unwrap().as_str().unwrap(),
        "unicity",
        "platform should be the lowercased channel name"
    );

    // Check capabilities.uplink = true
    let caps = parsed.get("capabilities").unwrap();
    assert_eq!(
        caps.get("uplink").unwrap().as_bool().unwrap(),
        true,
        "capabilities.uplink should be true when channels are present"
    );
}

#[test]
fn tier2_known_platform_channel_serializes_as_string() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_channel_plugin(plugin_dir.path(), &["discord"], serde_json::json!({}));

    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: false,
        no_cache: true,
    })
    .unwrap();

    let capsule_toml = std::fs::read_to_string(output_dir.path().join("Capsule.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&capsule_toml).unwrap();

    let uplinks = parsed.get("uplink").unwrap().as_array().unwrap();
    let platform = &uplinks[0].get("platform").unwrap();
    // Known platform should serialize as a bare string, not a table
    assert_eq!(
        platform.as_str().unwrap(),
        "discord",
        "known platform should serialize as bare string \"discord\""
    );
}

#[test]
fn tier2_uihints_sensitive_overrides_secret_detection() {
    let plugin_dir = tempfile::tempdir().unwrap();
    // "myCredential" is NOT caught by is_secret_key(), but uiHints marks it sensitive
    create_channel_plugin(
        plugin_dir.path(),
        &[],
        serde_json::json!({ "myCredential": { "sensitive": true } }),
    );

    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: false,
        no_cache: true,
    })
    .unwrap();

    let capsule_toml = std::fs::read_to_string(output_dir.path().join("Capsule.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&capsule_toml).unwrap();

    let env = parsed.get("env").unwrap();
    let cred = env.get("myCredential").unwrap();
    assert_eq!(
        cred.get("type").unwrap().as_str().unwrap(),
        "secret",
        "myCredential should be secret via uiHints.sensitive, got:\n{capsule_toml}"
    );
}

#[test]
fn tier2_uihints_label_used_as_request() {
    let plugin_dir = tempfile::tempdir().unwrap();
    create_channel_plugin(
        plugin_dir.path(),
        &[],
        serde_json::json!({ "network": { "label": "Select Network" } }),
    );

    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    compile_plugin(&CompileOptions {
        plugin_dir: plugin_dir.path(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: false,
        no_cache: true,
    })
    .unwrap();

    let capsule_toml = std::fs::read_to_string(output_dir.path().join("Capsule.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&capsule_toml).unwrap();

    let env = parsed.get("env").unwrap();
    let network = env.get("network").unwrap();
    assert_eq!(
        network.get("request").unwrap().as_str().unwrap(),
        "Select Network",
        "request should use uiHints label"
    );
}
