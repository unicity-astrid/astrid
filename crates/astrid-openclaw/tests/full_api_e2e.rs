//! End-to-end test for the full plugin API surface.
//!
//! Compiles the `test-full-api` fixture through both Tier 1 (WASM shim)
//! and Tier 2 (Node.js bridge) paths. Verifies that every registration
//! method, hook, event handler, and host function binding is correctly
//! captured in the generated output.

use std::collections::HashMap;
use std::path::Path;

use astrid_openclaw::pipeline::{CompileOptions, compile_plugin};
use astrid_openclaw::shim;
use astrid_openclaw::tier::PluginTier;

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-full-api")
}

fn fixture_source() -> String {
    std::fs::read_to_string(fixture_dir().join("index.js"))
        .expect("test-full-api/index.js should exist")
}

fn fixture_identity() -> shim::PluginIdentity<'static> {
    shim::PluginIdentity {
        id: "test-full-api",
        name: Some("Full API Surface Test"),
        version: Some("1.0.0"),
        description: Some("Exercises every registration method and hook in the plugin API"),
    }
}

// ── Tier 1: WASM shim generation ──────────────────────────────────────

#[test]
fn tier1_shim_captures_all_registration_methods() {
    let source = fixture_source();
    let mut config = HashMap::new();
    config.insert("debug".into(), serde_json::json!(true));
    config.insert("api_key".into(), serde_json::json!("sk-test"));

    let shim = shim::generate(&source, &config, &fixture_identity());

    // ── registerTool (both forms) ─────────────────────────────────────
    assert!(
        shim.contains("tool-string-form"),
        "missing tool registered with string name form"
    );
    assert!(
        shim.contains("tool-object-form"),
        "missing tool registered with object definition form"
    );
    assert!(shim.contains("run-diagnostics"), "missing diagnostics tool");

    // ── registerService ───────────────────────────────────────────────
    assert!(
        shim.contains("background-worker"),
        "missing registerService capture"
    );

    // ── registerChannel (both forms) ──────────────────────────────────
    assert!(
        shim.contains("notifications"),
        "missing channel registered with string name form"
    );
    assert!(
        shim.contains("alerts"),
        "missing channel registered with object definition form"
    );

    // ── registerHook ──────────────────────────────────────────────────
    assert!(
        shim.contains("session_start"),
        "missing session_start hook registration"
    );
    assert!(
        shim.contains("before_tool_call"),
        "missing before_tool_call hook registration"
    );
    assert!(
        shim.contains("after_tool_call"),
        "missing after_tool_call hook registration"
    );
    assert!(
        shim.contains("session_end"),
        "missing session_end hook registration"
    );

    // ── registerCommand ───────────────────────────────────────────────
    assert!(
        shim.contains("reload-config"),
        "missing registerCommand capture"
    );

    // ── registerGatewayMethod ─────────────────────────────────────────
    assert!(
        shim.contains("ping"),
        "missing registerGatewayMethod capture"
    );

    // ── registerHttpHandler ───────────────────────────────────────────
    assert!(
        shim.contains("/webhook"),
        "missing registerHttpHandler capture"
    );

    // ── registerHttpRoute ─────────────────────────────────────────────
    assert!(
        shim.contains("/api/data"),
        "missing registerHttpRoute capture"
    );

    // ── registerProvider ──────────────────────────────────────────────
    assert!(
        shim.contains("oauth-github"),
        "missing registerProvider capture"
    );

    // ── registerCli ───────────────────────────────────────────────────
    assert!(shim.contains("export"), "missing registerCli capture");

    // ── on (event handlers) ───────────────────────────────────────────
    assert!(
        shim.contains("message_received"),
        "missing on('message_received') handler"
    );
    assert!(
        shim.contains("message_sending"),
        "missing on('message_sending') handler"
    );
    assert!(
        shim.contains("prompt_building"),
        "missing on('prompt_building') handler"
    );
    assert!(
        shim.contains("model_resolving"),
        "missing on('model_resolving') handler"
    );
    assert!(
        shim.contains("context_compaction_started"),
        "missing on('context_compaction_started') handler"
    );
    assert!(
        shim.contains("tool_result_persisting"),
        "missing on('tool_result_persisting') handler"
    );
}

#[test]
fn tier1_shim_has_all_host_function_bindings() {
    let source = fixture_source();
    let config = HashMap::new();
    let shim = shim::generate(&source, &config, &fixture_identity());

    // Host function references
    for hf in &[
        "astrid_log",
        "astrid_get_config",
        "astrid_kv_get",
        "astrid_kv_set",
        "astrid_read_file",
        "astrid_write_file",
        "astrid_http_request",
    ] {
        assert!(shim.contains(hf), "missing host function reference: {hf}");
    }

    // Host function wrappers
    for wrapper in &[
        "hostLog",
        "hostGetConfig",
        "hostKvGet",
        "hostKvSet",
        "hostReadFile",
        "hostWriteFile",
        "hostHttpRequest",
    ] {
        assert!(
            shim.contains(wrapper),
            "missing host function wrapper: {wrapper}"
        );
    }
}

#[test]
fn tier1_shim_has_extism_exports() {
    let source = fixture_source();
    let config = HashMap::new();
    let shim = shim::generate(&source, &config, &fixture_identity());

    assert!(
        shim.contains("describe-tools"),
        "missing describe-tools export"
    );
    assert!(
        shim.contains("astrid_tool_call"),
        "missing astrid_tool_call export"
    );
    assert!(
        shim.contains("astrid_hook_trigger"),
        "missing astrid_hook_trigger export"
    );
}

#[test]
fn tier1_shim_has_node_polyfills() {
    let source = fixture_source();
    let config = HashMap::new();
    let shim = shim::generate(&source, &config, &fixture_identity());

    // Core polyfills that must exist for compatibility
    for module_name in &[
        "\"fs\"",
        "\"path\"",
        "\"os\"",
        "\"buffer\"",
        "\"crypto\"",
        "\"url\"",
        "\"events\"",
        "\"util\"",
        "\"stream\"",
        "\"http\"",
        "\"https\"",
        "\"querystring\"",
        "\"assert\"",
        "\"string_decoder\"",
    ] {
        assert!(
            shim.contains(module_name),
            "missing Node.js polyfill module: {module_name}"
        );
    }

    // Global stubs
    assert!(shim.contains("setTimeout"), "missing setTimeout polyfill");
    assert!(shim.contains("setInterval"), "missing setInterval polyfill");
    assert!(shim.contains("process"), "missing process global");
    assert!(shim.contains("Buffer"), "missing Buffer global");
    assert!(shim.contains("require.resolve"), "missing require.resolve");
}

// ── Tier 1: Full pipeline compilation ─────────────────────────────────

#[test]
fn tier1_pipeline_compiles_full_api_plugin() {
    let output_dir = tempfile::tempdir().unwrap();
    let config = HashMap::new();

    let result = compile_plugin(&CompileOptions {
        plugin_dir: &fixture_dir(),
        output_dir: output_dir.path(),
        config: &config,
        cache_dir: None,
        js_only: true,
        no_cache: true,
    })
    .unwrap();

    assert_eq!(result.astrid_id, "test-full-api");
    assert_eq!(result.tier, PluginTier::Wasm);

    // Shim file was generated
    let shim_path = output_dir.path().join("shim.js");
    assert!(shim_path.exists(), "shim.js should be generated");

    let shim = std::fs::read_to_string(&shim_path).unwrap();
    assert!(shim.contains("test-full-api activating"));
}

// ── Tier 2: Node.js bridge pipeline ───────────────────────────────────

#[test]
fn tier2_pipeline_generates_correct_capsule_toml() {
    // Create a Tier 2 variant (add package.json with deps)
    let plugin_dir = tempfile::tempdir().unwrap();

    // Copy manifest
    std::fs::write(
        plugin_dir.path().join("openclaw.plugin.json"),
        std::fs::read_to_string(fixture_dir().join("openclaw.plugin.json")).unwrap(),
    )
    .unwrap();

    // Add package.json with npm deps → forces Tier 2
    std::fs::write(
        plugin_dir.path().join("package.json"),
        r#"{"name":"test-full-api","version":"1.0.0","dependencies":{"lodash":"^4.0.0"}}"#,
    )
    .unwrap();

    // Copy source
    std::fs::create_dir_all(plugin_dir.path().join("src")).unwrap();
    std::fs::write(plugin_dir.path().join("src/index.js"), fixture_source()).unwrap();

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
    assert_eq!(result.astrid_id, "test-full-api");

    // Verify Capsule.toml structure
    let capsule_toml = std::fs::read_to_string(output_dir.path().join("Capsule.toml")).unwrap();

    // Package
    assert!(capsule_toml.contains(r#"name = "test-full-api""#));
    assert!(capsule_toml.contains(r#"version = "1.0.0""#));

    // MCP server config with bridge flags
    assert!(capsule_toml.contains("[[mcp_server]]"));
    assert!(capsule_toml.contains(r#"command = "node""#));
    assert!(capsule_toml.contains("astrid_bridge.mjs"));
    assert!(capsule_toml.contains("--entry"), "must pass --entry flag");
    assert!(
        capsule_toml.contains("--plugin-id"),
        "must pass --plugin-id flag"
    );

    // Capabilities
    assert!(capsule_toml.contains(r#"host_process = ["node"]"#));

    // Bridge script copied
    assert!(
        output_dir.path().join("astrid_bridge.mjs").exists(),
        "bridge script should be copied to output"
    );
}

// ── Event/Hook coverage verification ──────────────────────────────────

#[test]
fn all_astrid_events_have_event_type_strings() {
    use astrid_events::AstridEvent;

    // Construct every variant and verify event_type() returns a non-empty string.
    // This is a compile-time completeness check — if a new variant is added
    // without an event_type() arm, the match will be non-exhaustive.
    let events: Vec<AstridEvent> = vec![
        AstridEvent::RuntimeStarted {
            metadata: m(),
            version: s(),
        },
        AstridEvent::RuntimeStopped {
            metadata: m(),
            reason: None,
        },
        AstridEvent::AgentStarted {
            metadata: m(),
            agent_id: u(),
            agent_name: s(),
        },
        AstridEvent::AgentStopped {
            metadata: m(),
            agent_id: u(),
            reason: None,
        },
        AstridEvent::SessionCreated {
            metadata: m(),
            session_id: u(),
        },
        AstridEvent::SessionEnded {
            metadata: m(),
            session_id: u(),
            reason: None,
        },
        AstridEvent::SessionResumed {
            metadata: m(),
            session_id: u(),
        },
        AstridEvent::PromptBuilding {
            metadata: m(),
            request_id: u(),
        },
        AstridEvent::MessageSending {
            metadata: m(),
            message_id: u(),
            frontend: s(),
        },
        AstridEvent::ContextCompactionStarted {
            metadata: m(),
            session_id: u(),
            message_count: 0,
        },
        AstridEvent::ContextCompactionCompleted {
            metadata: m(),
            session_id: u(),
            messages_remaining: 0,
        },
        AstridEvent::SessionResetting {
            metadata: m(),
            session_id: u(),
        },
        AstridEvent::ModelResolving {
            metadata: m(),
            request_id: u(),
            provider: None,
            model: None,
        },
        AstridEvent::AgentLoopCompleted {
            metadata: m(),
            agent_id: u(),
            turns: 0,
            duration_ms: 0,
        },
        AstridEvent::ToolResultPersisting {
            metadata: m(),
            call_id: u(),
            tool_name: s(),
        },
        AstridEvent::MessageReceived {
            metadata: m(),
            message_id: u(),
            frontend: s(),
        },
        AstridEvent::MessageSent {
            metadata: m(),
            message_id: u(),
            frontend: s(),
        },
        AstridEvent::MessageProcessed {
            metadata: m(),
            message_id: u(),
            duration_ms: 0,
        },
        AstridEvent::LlmRequestStarted {
            metadata: m(),
            request_id: u(),
            provider: s(),
            model: s(),
        },
        AstridEvent::LlmRequestCompleted {
            metadata: m(),
            request_id: u(),
            success: true,
            input_tokens: None,
            output_tokens: None,
            duration_ms: 0,
        },
        AstridEvent::LlmStreamStarted {
            metadata: m(),
            request_id: u(),
            model: s(),
        },
        AstridEvent::LlmStreamChunk {
            metadata: m(),
            request_id: u(),
            chunk_index: 0,
            token_count: 0,
        },
        AstridEvent::LlmStreamCompleted {
            metadata: m(),
            request_id: u(),
            input_tokens: None,
            output_tokens: None,
            duration_ms: 0,
        },
        AstridEvent::ToolCallStarted {
            metadata: m(),
            call_id: u(),
            tool_name: s(),
            server_name: None,
        },
        AstridEvent::ToolCallCompleted {
            metadata: m(),
            call_id: u(),
            tool_name: s(),
            duration_ms: 0,
        },
        AstridEvent::ToolCallFailed {
            metadata: m(),
            call_id: u(),
            tool_name: s(),
            error: s(),
            duration_ms: 0,
        },
        AstridEvent::McpServerConnected {
            metadata: m(),
            server_name: s(),
            protocol_version: s(),
        },
        AstridEvent::McpServerDisconnected {
            metadata: m(),
            server_name: s(),
            reason: None,
        },
        AstridEvent::McpToolCalled {
            metadata: m(),
            server_name: s(),
            tool_name: s(),
            arguments: None,
        },
        AstridEvent::McpToolCompleted {
            metadata: m(),
            server_name: s(),
            tool_name: s(),
            success: true,
            duration_ms: 0,
        },
        AstridEvent::SubAgentSpawned {
            metadata: m(),
            subagent_id: u(),
            parent_id: u(),
            task: s(),
            depth: 0,
        },
        AstridEvent::SubAgentProgress {
            metadata: m(),
            subagent_id: u(),
            message: s(),
        },
        AstridEvent::SubAgentCompleted {
            metadata: m(),
            subagent_id: u(),
            duration_ms: 0,
        },
        AstridEvent::SubAgentFailed {
            metadata: m(),
            subagent_id: u(),
            error: s(),
            duration_ms: 0,
        },
        AstridEvent::SubAgentCancelled {
            metadata: m(),
            subagent_id: u(),
            reason: None,
        },
        AstridEvent::PluginLoaded {
            metadata: m(),
            plugin_id: s(),
            plugin_name: s(),
        },
        AstridEvent::PluginFailed {
            metadata: m(),
            plugin_id: s(),
            error: s(),
        },
        AstridEvent::PluginUnloaded {
            metadata: m(),
            plugin_id: s(),
            plugin_name: s(),
        },
        AstridEvent::CapabilityGranted {
            metadata: m(),
            capability_id: u(),
            resource: s(),
            action: s(),
        },
        AstridEvent::CapabilityRevoked {
            metadata: m(),
            capability_id: u(),
            reason: None,
        },
        AstridEvent::CapabilityChecked {
            metadata: m(),
            resource: s(),
            action: s(),
            allowed: true,
        },
        AstridEvent::AuthorizationDenied {
            metadata: m(),
            resource: s(),
            action: s(),
            reason: s(),
        },
        AstridEvent::SecurityViolation {
            metadata: m(),
            violation_type: s(),
            details: s(),
        },
        AstridEvent::ApprovalRequested {
            metadata: m(),
            request_id: u(),
            resource: s(),
            action: s(),
            description: s(),
        },
        AstridEvent::ApprovalGranted {
            metadata: m(),
            request_id: u(),
            duration: None,
        },
        AstridEvent::ApprovalDenied {
            metadata: m(),
            request_id: u(),
            reason: None,
        },
        AstridEvent::BudgetAllocated {
            metadata: m(),
            budget_id: u(),
            amount_cents: 0,
            currency: s(),
        },
        AstridEvent::BudgetWarning {
            metadata: m(),
            budget_id: u(),
            remaining_cents: 0,
            percent_used: 0.0,
        },
        AstridEvent::BudgetExceeded {
            metadata: m(),
            budget_id: u(),
            overage_cents: 0,
        },
        AstridEvent::KernelStarted {
            metadata: m(),
            version: s(),
        },
        AstridEvent::KernelShutdown {
            metadata: m(),
            reason: None,
        },
        AstridEvent::ConfigReloaded { metadata: m() },
        AstridEvent::ConfigChanged {
            metadata: m(),
            key: s(),
        },
        AstridEvent::HealthCheckCompleted {
            metadata: m(),
            healthy: true,
            checks_performed: 0,
            checks_failed: 0,
        },
        AstridEvent::AuditEntryCreated {
            metadata: m(),
            entry_id: u(),
            entry_type: s(),
        },
        AstridEvent::ErrorOccurred {
            metadata: m(),
            code: s(),
            message: s(),
            stack_trace: None,
        },
        AstridEvent::Ipc {
            metadata: m(),
            message: astrid_events::IpcMessage::new(
                "test",
                astrid_events::IpcPayload::Custom {
                    data: serde_json::json!({}),
                },
                uuid::Uuid::nil(),
            ),
        },
        AstridEvent::Custom {
            metadata: m(),
            name: s(),
            data: serde_json::json!({}),
        },
    ];

    for event in &events {
        let et = event.event_type();
        assert!(
            !et.is_empty(),
            "event_type() must not be empty for {:?}",
            std::mem::discriminant(event)
        );
        // Verify metadata() doesn't panic
        let _ = event.metadata();
    }

    // If this test compiles, every variant has a matching arm in event_type()
    // and metadata(). Any new variant without one would cause a compile error.
}

// ── Helpers ───────────────────────────────────────────────────────────

fn m() -> astrid_events::EventMetadata {
    astrid_events::EventMetadata::new("test")
}

fn u() -> uuid::Uuid {
    uuid::Uuid::nil()
}

fn s() -> String {
    String::new()
}
