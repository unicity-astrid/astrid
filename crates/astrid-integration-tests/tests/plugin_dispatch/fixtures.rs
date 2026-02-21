//! Shared test plugin fixtures for plugin dispatch integration tests.

use std::collections::HashMap;
use std::sync::Arc;

use astrid_plugins::context::{PluginContext, PluginToolContext};
use astrid_plugins::error::{PluginError, PluginResult};
use astrid_plugins::manifest::{PluginEntryPoint, PluginManifest};
use astrid_plugins::plugin::{Plugin, PluginId, PluginState};
use astrid_plugins::tool::PluginTool;

// ---------------------------------------------------------------------------
// Test plugin that provides an "echo" tool
// ---------------------------------------------------------------------------

pub struct TestPlugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub tools: Vec<Arc<dyn PluginTool>>,
}

impl TestPlugin {
    pub fn new(id: &str) -> Self {
        let plugin_id = PluginId::from_static(id);
        Self {
            manifest: PluginManifest {
                id: plugin_id.clone(),
                name: format!("Test Plugin {id}"),
                version: "0.1.0".into(),
                description: None,
                author: None,
                entry_point: PluginEntryPoint::Wasm {
                    path: "plugin.wasm".into(),
                    hash: None,
                },
                capabilities: vec![],
                connectors: vec![],
                config: HashMap::new(),
            },
            id: plugin_id,
            state: PluginState::Ready,
            tools: vec![Arc::new(EchoTool)],
        }
    }
}

#[async_trait::async_trait]
impl Plugin for TestPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn state(&self) -> PluginState {
        self.state.clone()
    }
    async fn load(&mut self, _ctx: &PluginContext) -> PluginResult<()> {
        self.state = PluginState::Ready;
        Ok(())
    }
    async fn unload(&mut self) -> PluginResult<()> {
        self.state = PluginState::Unloaded;
        Ok(())
    }
    fn tools(&self) -> &[Arc<dyn PluginTool>] {
        &self.tools
    }
}

// ---------------------------------------------------------------------------
// EchoTool: echoes the input message
// ---------------------------------------------------------------------------

pub struct EchoTool;

#[async_trait::async_trait]
impl PluginTool for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn description(&self) -> &'static str {
        "Echoes the input message"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &PluginToolContext,
    ) -> PluginResult<String> {
        let msg = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("no message");
        Ok(format!("echo: {msg}"))
    }
}

// ---------------------------------------------------------------------------
// FailingTool: always returns an error
// ---------------------------------------------------------------------------

pub struct FailingTool;

#[async_trait::async_trait]
impl PluginTool for FailingTool {
    fn name(&self) -> &'static str {
        "fail"
    }
    fn description(&self) -> &'static str {
        "Always fails"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &PluginToolContext,
    ) -> PluginResult<String> {
        Err(PluginError::ExecutionFailed(
            "intentional test failure".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// UpperTool: uppercases the input (used in multi-plugin dispatch tests)
// ---------------------------------------------------------------------------

pub struct UpperTool;

#[async_trait::async_trait]
impl PluginTool for UpperTool {
    fn name(&self) -> &'static str {
        "upper"
    }
    fn description(&self) -> &'static str {
        "Uppercases the input"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            }
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &PluginToolContext,
    ) -> PluginResult<String> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("nothing");
        Ok(text.to_uppercase())
    }
}

// ---------------------------------------------------------------------------
// Helper to build a TestPlugin with custom tools
// ---------------------------------------------------------------------------

pub fn make_plugin(id: &str, tools: Vec<Arc<dyn PluginTool>>) -> TestPlugin {
    let plugin_id = PluginId::from_static(id);
    TestPlugin {
        manifest: PluginManifest {
            id: plugin_id.clone(),
            name: format!("Test Plugin {id}"),
            version: "0.1.0".into(),
            description: None,
            author: None,
            entry_point: PluginEntryPoint::Wasm {
                path: "plugin.wasm".into(),
                hash: None,
            },
            capabilities: vec![],
            connectors: vec![],
            config: HashMap::new(),
        },
        id: plugin_id,
        state: PluginState::Ready,
        tools,
    }
}
