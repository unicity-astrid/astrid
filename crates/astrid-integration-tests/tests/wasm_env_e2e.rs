use std::path::PathBuf;
use std::sync::Arc;

use astrid_capsule::context::{CapsuleContext, CapsuleToolContext};
use astrid_capsule::loader::CapsuleLoader;
use astrid_capsule::manifest::{
    CapabilitiesDef, CapsuleManifest, ComponentDef, PackageDef, ToolDef,
};
use astrid_events::EventBus;
use astrid_storage::{MemoryKvStore, ScopedKvStore};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn test_wasm_capsule_e2e_env_config_injection() {
    let tools = vec![ToolDef {
        name: "test-config".into(),
        description: "Test config tool".into(),
        input_schema: json!({ "type": "object" }),
    }];

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("test-all-endpoints.wasm");

    if !fixture_path.exists() {
        eprintln!(
            "Skipping test: Fixture not found at {}",
            fixture_path.display()
        );
        return;
    }

    let mut env = std::collections::HashMap::new();
    env.insert(
        "test_key".into(),
        astrid_capsule::manifest::EnvDef {
            env_type: "string".into(),
            default: Some(json!("default_value")),
            request: None,
            description: None,
        },
    );
    env.insert(
        "injected_key".into(),
        astrid_capsule::manifest::EnvDef {
            env_type: "secret".into(),
            default: None,
            request: None,
            description: None,
        },
    );

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
            net: vec![],
            kv: vec!["*".into()],
            fs_read: vec![],
            fs_write: vec![],
            host_process: vec![],
        },
        env,
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
        .unwrap();

    let kv = ScopedKvStore::new(Arc::new(MemoryKvStore::new()), "test-plugin").unwrap();
    // Inject a value into the KV store for the engine to read
    kv.set("injected_key", b"injected_value".to_vec())
        .await
        .unwrap();

    let event_bus = Arc::new(EventBus::with_capacity(128));
    let ctx = CapsuleContext::new(
        std::env::current_dir().unwrap(),
        kv.clone(),
        event_bus.clone(),
    );

    capsule.load(&ctx).await.unwrap();

    let tool_ctx = CapsuleToolContext::new(
        capsule.id().clone(),
        std::env::current_dir().unwrap(),
        kv.clone(),
    );

    let tools_list = capsule.tools();
    let config_tool = tools_list
        .iter()
        .find(|t| t.name() == "test-config")
        .unwrap();

    // 1. Read default value
    let res1 = config_tool
        .execute(json!({ "key": "test_key" }), &tool_ctx)
        .await
        .unwrap();
    let out1_outer: serde_json::Value = serde_json::from_str(&res1).unwrap();
    let out1: serde_json::Value =
        serde_json::from_str(out1_outer["content"].as_str().unwrap()).unwrap();
    assert_eq!(out1["found"], true);
    assert_eq!(out1["value"], "default_value");

    // 2. Read injected KV value
    let res2 = config_tool
        .execute(json!({ "key": "injected_key" }), &tool_ctx)
        .await
        .unwrap();
    let out2_outer: serde_json::Value = serde_json::from_str(&res2).unwrap();
    let out2: serde_json::Value =
        serde_json::from_str(out2_outer["content"].as_str().unwrap()).unwrap();
    assert_eq!(out2["found"], true);
    assert_eq!(out2["value"], "injected_value");

    // 3. Read missing value
    let res3 = config_tool
        .execute(json!({ "key": "missing_key" }), &tool_ctx)
        .await
        .unwrap();
    let out3_outer: serde_json::Value = serde_json::from_str(&res3).unwrap();
    let out3: serde_json::Value =
        serde_json::from_str(out3_outer["content"].as_str().unwrap()).unwrap();
    assert_eq!(out3["found"], false);
}
