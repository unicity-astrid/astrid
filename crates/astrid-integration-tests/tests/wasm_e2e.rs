use std::path::PathBuf;
use std::sync::Arc;

use astrid_capsule::capsule::CapsuleState;
use astrid_capsule::context::{CapsuleContext, CapsuleToolContext};
use astrid_capsule::loader::CapsuleLoader;
use astrid_capsule::manifest::{
    CapabilitiesDef, CapsuleManifest, ComponentDef, PackageDef, ToolDef,
};
use astrid_events::EventBus;
use astrid_storage::{MemoryKvStore, ScopedKvStore};
use serde_json::json;

async fn setup_test_capsule(
    tools: Vec<ToolDef>,
    fs_read_caps: Vec<String>,
    fs_write_caps: Vec<String>,
    net_caps: Vec<String>,
) -> Option<(
    Box<dyn astrid_capsule::capsule::Capsule>,
    CapsuleToolContext,
    tempfile::TempDir,
)> {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("test-all-endpoints.wasm");

    if !fixture_path.exists() {
        eprintln!(
            "Skipping test: Fixture not found at {}",
            fixture_path.display()
        );
        return None;
    }

    let manifest = CapsuleManifest {
        package: PackageDef {
            name: "test-plugin".into(),
            version: "0.1.0".into(),
            description: None,
            authors: vec![],
            repository: None,
            homepage: None,
            documentation: None,
            license: None,
            license_file: None,
            readme: None,
            keywords: vec![],
            categories: vec![],
            astrid_version: None,
            publish: None,
            include: None,
            exclude: None,
            metadata: None,
        },
        component: Some(ComponentDef {
            entrypoint: fixture_path.clone(),
            hash: None,
        }),
        dependencies: Default::default(),
        capabilities: CapabilitiesDef {
            net: net_caps,
            kv: vec!["*".into()],
            fs_read: fs_read_caps,
            fs_write: fs_write_caps,
            host_process: vec![],
        },
        env: Default::default(),
        context_files: vec![],
        commands: vec![],
        mcp_servers: vec![],
        skills: vec![],
        uplinks: vec![],
        llm_providers: vec![],
        interceptors: vec![],
        cron_jobs: vec![],
        tools,
    };

    let mcp_client = astrid_mcp::McpClient::with_config(Default::default());
    let loader = CapsuleLoader::new(mcp_client);

    let mut capsule = loader
        .create_capsule(manifest, fixture_path.parent().unwrap().to_path_buf())
        .expect("Failed to create capsule");

    let temp_workspace = tempfile::tempdir().unwrap();

    let kv = ScopedKvStore::new(Arc::new(MemoryKvStore::new()), "test-plugin").unwrap();
    let event_bus = Arc::new(EventBus::with_capacity(128));
    let ctx = CapsuleContext::new(
        temp_workspace.path().to_path_buf(),
        kv.clone(),
        event_bus.clone(),
    );

    capsule.load(&ctx).await.expect("Failed to load capsule");
    assert_eq!(capsule.state(), CapsuleState::Ready);

    let tool_ctx = CapsuleToolContext::new(
        capsule.id().clone(),
        temp_workspace.path().to_path_buf(),
        kv.clone(),
    );

    Some((capsule, tool_ctx, temp_workspace))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_basic_log() {
    let tools = vec![ToolDef {
        name: "test-log".into(),
        description: "Test log tool".into(),
        input_schema: json!({ "type": "object", "properties": { "message": { "type": "string" } } }),
    }];
    let Some((capsule, tool_ctx, _tmp)) =
        setup_test_capsule(tools, vec!["/".into()], vec!["/".into()], vec!["*".into()]).await
    else {
        return;
    };

    let test_log_tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-log")
        .unwrap();
    let result = test_log_tool
        .execute(json!({ "message": "hello integration test" }), &tool_ctx)
        .await
        .expect("Tool execution failed");

    let output: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(output["is_error"], false);
    assert_eq!(
        output["content"],
        "logged at all levels: hello integration test"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_malicious_log_rejected() {
    let tools = vec![ToolDef {
        name: "test-malicious-log".into(),
        description: "Malicious log tool".into(),
        input_schema: json!({ "type": "object" }),
    }];
    let Some((capsule, tool_ctx, _tmp)) =
        setup_test_capsule(tools, vec!["/".into()], vec!["/".into()], vec!["*".into()]).await
    else {
        return;
    };

    let malicious_tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-malicious-log")
        .unwrap();
    let result = malicious_tool.execute(json!({}), &tool_ctx).await;

    // The WASM runtime should trap and return a CapsuleError::WasmError
    // because the memory allocation exceeded the 64KB log limit defined in `host/util.rs`.
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("exceeds maximum allowed limit"),
        "Actual error: {err_str}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_malicious_kv_rejected() {
    let tools = vec![ToolDef {
        name: "test-malicious-kv".into(),
        description: "Malicious kv tool".into(),
        input_schema: json!({ "type": "object" }),
    }];
    let Some((capsule, tool_ctx, _tmp)) =
        setup_test_capsule(tools, vec!["/".into()], vec!["/".into()], vec!["*".into()]).await
    else {
        return;
    };

    let malicious_tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-malicious-kv")
        .unwrap();
    let result = malicious_tool.execute(json!({}), &tool_ctx).await;

    // The WASM runtime should trap and return a CapsuleError::WasmError
    // because the memory allocation exceeded the 10MB KV limit.
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("exceeds maximum allowed limit"),
        "Actual error: {err_str}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_ipc_limits() {
    let tools = vec![ToolDef {
        name: "test-ipc-limits".into(),
        description: "Test IPC Limits".into(),
        input_schema: json!({ "type": "object" }),
    }];
    let Some((capsule, tool_ctx, _tmp)) =
        setup_test_capsule(tools, vec!["/".into()], vec!["/".into()], vec!["*".into()]).await
    else {
        return;
    };

    let tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-ipc-limits")
        .unwrap();

    // Test 1: Publish large payload
    let result_large = tool
        .execute(json!({ "test_type": "publish_large" }), &tool_ctx)
        .await;
    assert!(result_large.is_err());
    let err_str = result_large.unwrap_err().to_string();
    assert!(
        err_str.contains("Payload too large"),
        "Actual error: {err_str}"
    );

    // Test 2: Subscription loop (more than 128)
    let result_loop = tool
        .execute(json!({ "test_type": "subscribe_loop" }), &tool_ctx)
        .await;
    assert!(
        result_loop.is_err(),
        "Expected loop to trap but it succeeded: {result_loop:?}"
    );
    let err_str = result_loop.unwrap_err().to_string();
    assert!(
        err_str.contains("Subscription limit reached"),
        "Actual error: {err_str}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_vfs_path_traversal() {
    let tools = vec![ToolDef {
        name: "test-file-read".into(),
        description: "Test file read tool".into(),
        input_schema: json!({ "type": "object" }),
    }];
    // Only give access to the workspace root
    let Some((capsule, tool_ctx, _tmp)) =
        setup_test_capsule(tools, vec!["/".into()], vec!["/".into()], vec!["*".into()]).await
    else {
        return;
    };

    let tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-file-read")
        .unwrap();

    // Attempt path traversal out of the workspace boundary
    let result = tool
        .execute(json!({ "path": "../../../../../../etc/passwd" }), &tool_ctx)
        .await;

    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("escapes workspace boundary"),
        "Actual error: {err_str}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_http_security_gate() {
    let tools = vec![ToolDef {
        name: "test-http".into(),
        description: "Test http tool".into(),
        input_schema: json!({ "type": "object" }),
    }];

    // Only give access to api.github.com
    let Some((capsule, tool_ctx, _tmp)) =
        setup_test_capsule(tools, vec![], vec![], vec!["api.github.com".into()]).await
    else {
        return;
    };

    let tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-http")
        .unwrap();

    // Attempt HTTP request to an allowed domain
    // This will likely fail with a connection error or 404, but NOT a security denied error
    let allowed_req = json!({
        "method": "GET",
        "url": "https://api.github.com/v1/user",
        "headers": {},
        "body": null
    });
    let result_allowed = tool
        .execute(json!({ "request": allowed_req.to_string() }), &tool_ctx)
        .await;
    let err_str_allowed = result_allowed
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        !err_str_allowed.contains("security denied"),
        "Should not be blocked by security gate"
    );

    // Attempt HTTP request to a denied domain
    let denied_req = json!({
        "method": "GET",
        "url": "https://evil-hacker.com/steal",
        "headers": {},
        "body": null
    });
    let result_denied = tool
        .execute(json!({ "request": denied_req.to_string() }), &tool_ctx)
        .await;

    assert!(result_denied.is_err(), "Denied request should fail");
    let err_str_denied = result_denied.unwrap_err().to_string();
    assert!(
        err_str_denied.contains("security denied"),
        "Actual error: {err_str_denied}"
    );
    assert!(
        err_str_denied.contains("not declared in manifest"),
        "Actual error: {err_str_denied}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_malicious_http_headers() {
    let tools = vec![ToolDef {
        name: "test-malicious-http-headers".into(),
        description: "Malicious HTTP headers tool".into(),
        input_schema: json!({ "type": "object" }),
    }];
    let Some((capsule, tool_ctx, _tmp)) =
        setup_test_capsule(tools, vec![], vec![], vec!["*".into()]).await
    else {
        return;
    };

    let tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-malicious-http-headers")
        .unwrap();
    let result = tool.execute(json!({}), &tool_ctx).await;

    // The WASM runtime should trap and return a CapsuleError::WasmError
    // because reqwest rejects headers with newlines (CRLF injection prevention).
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("invalid HTTP header name") || err_str.contains("failed to parse header"),
        "Actual error: {err_str}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_vfs_legitimate_rw() {
    let tools = vec![
        ToolDef {
            name: "test-file-write".into(),
            description: "Write tool".into(),
            input_schema: json!({ "type": "object" }),
        },
        ToolDef {
            name: "test-file-read".into(),
            description: "Read tool".into(),
            input_schema: json!({ "type": "object" }),
        },
    ];
    let Some((capsule, tool_ctx, _temp_dir)) =
        setup_test_capsule(tools, vec!["/".into()], vec!["/".into()], vec!["*".into()]).await
    else {
        return;
    };

    let write_tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-file-write")
        .unwrap();
    let read_tool = capsule
        .tools()
        .iter()
        .find(|t| t.name() == "test-file-read")
        .unwrap();

    // Write a test file into the workspace root
    let file_path_str = "test_rw_legitimate.txt";

    // Write
    let w_res = write_tool
        .execute(
            json!({ "path": &file_path_str, "content": "hello vfs" }),
            &tool_ctx,
        )
        .await;
    assert!(w_res.is_ok(), "Write failed: {w_res:?}");

    // Read
    let r_res = read_tool
        .execute(json!({ "path": &file_path_str }), &tool_ctx)
        .await;
    assert!(r_res.is_ok(), "Read failed: {r_res:?}");

    let output: serde_json::Value = serde_json::from_str(&r_res.unwrap()).unwrap();
    let inner: serde_json::Value =
        serde_json::from_str(output["content"].as_str().unwrap()).unwrap();
    assert_eq!(inner["content"], "hello vfs");

    // Cleanup
    let _ = std::fs::remove_file(file_path_str);
}
