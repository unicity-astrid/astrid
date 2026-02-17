//! Plugin registry.
//!
//! Manages the set of loaded plugins and provides tool lookup across
//! all registered plugins.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{debug, info};

use crate::error::{PluginError, PluginResult};
use crate::plugin::{Plugin, PluginId};
use crate::tool::PluginTool;

/// Fully qualified tool name: `plugin:{plugin_id}:{tool_name}`.
///
/// This naming convention avoids collision with built-in tools (no colons)
/// and MCP tools (`server:tool` — single colon).
fn qualified_tool_name(plugin_id: &PluginId, tool_name: &str) -> String {
    format!("plugin:{plugin_id}:{tool_name}")
}

/// A tool definition exported for the LLM.
#[derive(Debug, Clone)]
pub struct PluginToolDefinition {
    /// Fully qualified tool name (`plugin:{plugin_id}:{tool_name}`).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for tool input.
    pub input_schema: serde_json::Value,
}

/// Registry of loaded plugins.
///
/// Parallel to `ToolRegistry` in `astrid-tools`. Stores plugins keyed by
/// their `PluginId` and provides cross-plugin tool lookup.
pub struct PluginRegistry {
    plugins: HashMap<PluginId, Box<dyn Plugin>>,
}

impl PluginRegistry {
    /// Create an empty plugin registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register a plugin.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::AlreadyRegistered`] if a plugin with the same
    /// ID is already in the registry.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> PluginResult<()> {
        let id = plugin.id().clone();
        if self.plugins.contains_key(&id) {
            return Err(PluginError::AlreadyRegistered(id));
        }
        info!(plugin_id = %id, "Registered plugin");
        self.plugins.insert(id, plugin);
        Ok(())
    }

    /// Unregister a plugin, returning it if it was present.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] if no plugin with the given ID exists.
    pub fn unregister(&mut self, id: &PluginId) -> PluginResult<Box<dyn Plugin>> {
        let plugin = self
            .plugins
            .remove(id)
            .ok_or_else(|| PluginError::NotFound(id.clone()))?;
        info!(plugin_id = %id, "Unregistered plugin");
        Ok(plugin)
    }

    /// Get a reference to a plugin by ID.
    #[must_use]
    pub fn get(&self, id: &PluginId) -> Option<&dyn Plugin> {
        self.plugins.get(id).map(AsRef::as_ref)
    }

    /// Get a mutable reference to a plugin by ID.
    #[must_use]
    pub fn get_mut(&mut self, id: &PluginId) -> Option<&mut Box<dyn Plugin>> {
        self.plugins.get_mut(id)
    }

    /// List all registered plugin IDs.
    #[must_use]
    pub fn list(&self) -> Vec<&PluginId> {
        self.plugins.keys().collect()
    }

    /// Number of registered plugins.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Find a tool by its fully qualified name (`plugin:{plugin_id}:{tool_name}`).
    ///
    /// Returns the plugin and an `Arc` clone of the tool. The `Arc` allows
    /// callers to drop the registry lock before executing the tool.
    #[must_use]
    pub fn find_tool(&self, qualified_name: &str) -> Option<(&dyn Plugin, Arc<dyn PluginTool>)> {
        // Parse "plugin:{plugin_id}:{tool_name}"
        let rest = qualified_name.strip_prefix("plugin:")?;
        let (plugin_id_str, tool_name) = rest.split_once(':')?;

        let plugin_id = PluginId::new(plugin_id_str).ok()?;
        let plugin = self.plugins.get(&plugin_id)?;

        let tool = plugin.tools().iter().find(|t| t.name() == tool_name)?;

        debug!(
            qualified_name,
            plugin_id = %plugin_id,
            tool_name,
            "Found plugin tool"
        );
        Some((plugin.as_ref(), Arc::clone(tool)))
    }

    /// Check if a tool name refers to a plugin tool (has two colons with `plugin:` prefix).
    #[must_use]
    pub fn is_plugin_tool(name: &str) -> bool {
        name.starts_with("plugin:") && name.matches(':').count() == 2
    }

    /// Export all tool definitions from all plugins for the LLM.
    #[must_use]
    pub fn all_tool_definitions(&self) -> Vec<PluginToolDefinition> {
        let mut defs = Vec::new();
        for (plugin_id, plugin) in &self.plugins {
            if !matches!(plugin.state(), crate::plugin::PluginState::Ready) {
                continue;
            }
            for tool in plugin.tools() {
                defs.push(PluginToolDefinition {
                    name: qualified_tool_name(plugin_id, tool.name()),
                    description: tool.description().to_string(),
                    input_schema: tool.input_schema(),
                });
            }
        }
        defs
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("plugin_count", &self.plugins.len())
            .field("plugin_ids", &self.list())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{PluginContext, PluginToolContext};
    use crate::manifest::{PluginEntryPoint, PluginManifest};
    use crate::plugin::PluginState;

    /// A test plugin that provides a single tool.
    struct TestPlugin {
        id: PluginId,
        manifest: PluginManifest,
        state: PluginState,
        tools: Vec<Arc<dyn PluginTool>>,
    }

    impl TestPlugin {
        fn new(id: &str) -> Self {
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
                    config: HashMap::new(),
                },
                id: plugin_id,
                state: PluginState::Ready,
                tools: vec![Arc::new(EchoTool)],
            }
        }

        fn with_no_tools(id: &str) -> Self {
            let mut p = Self::new(id);
            p.tools.clear();
            p
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

    struct EchoTool;

    #[async_trait::async_trait]
    impl PluginTool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes the input"
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
            Ok(args.to_string())
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut registry = PluginRegistry::new();
        assert!(registry.is_empty());

        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        assert_eq!(registry.len(), 1);

        let id = PluginId::from_static("alpha");
        assert!(registry.get(&id).is_some());
        assert_eq!(registry.get(&id).unwrap().id().as_str(), "alpha");
    }

    #[test]
    fn test_register_duplicate_fails() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        let result = registry.register(Box::new(TestPlugin::new("alpha")));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::AlreadyRegistered(_)
        ));
    }

    #[test]
    fn test_unregister() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();

        let id = PluginId::from_static("alpha");
        let plugin = registry.unregister(&id).unwrap();
        assert_eq!(plugin.id().as_str(), "alpha");
        assert!(registry.is_empty());
    }

    #[test]
    fn test_unregister_missing_fails() {
        let mut registry = PluginRegistry::new();
        let id = PluginId::from_static("missing");
        let result = registry.unregister(&id);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PluginError::NotFound(_)));
    }

    #[test]
    fn test_list_plugins() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        registry
            .register(Box::new(TestPlugin::new("beta")))
            .unwrap();

        let mut ids: Vec<&str> = registry.list().iter().map(|id| id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_find_tool() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();

        let result = registry.find_tool("plugin:alpha:echo");
        assert!(result.is_some());
        let (plugin, tool) = result.unwrap();
        assert_eq!(plugin.id().as_str(), "alpha");
        assert_eq!(tool.name(), "echo");
    }

    #[test]
    fn test_find_tool_missing_plugin() {
        let registry = PluginRegistry::new();
        assert!(registry.find_tool("plugin:missing:echo").is_none());
    }

    #[test]
    fn test_find_tool_missing_tool() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        assert!(registry.find_tool("plugin:alpha:missing").is_none());
    }

    #[test]
    fn test_find_tool_invalid_format() {
        let registry = PluginRegistry::new();
        assert!(registry.find_tool("builtin:echo").is_none());
        assert!(registry.find_tool("echo").is_none());
        assert!(registry.find_tool("").is_none());
    }

    #[test]
    fn test_is_plugin_tool() {
        assert!(PluginRegistry::is_plugin_tool("plugin:alpha:echo"));
        assert!(PluginRegistry::is_plugin_tool("plugin:my-plugin:read-file"));
        assert!(!PluginRegistry::is_plugin_tool("read_file"));
        assert!(!PluginRegistry::is_plugin_tool("server:tool"));
        assert!(!PluginRegistry::is_plugin_tool("plugin:only-one-colon"));
    }

    #[test]
    fn test_all_tool_definitions() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        registry
            .register(Box::new(TestPlugin::with_no_tools("beta")))
            .unwrap();

        let defs = registry.all_tool_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "plugin:alpha:echo");
        assert_eq!(defs[0].description, "Echoes the input");
    }

    #[test]
    fn test_all_tool_definitions_skips_non_ready_plugins() {
        let mut registry = PluginRegistry::new();

        // Register a Ready plugin and a non-Ready plugin (both have tools).
        let mut failed_plugin = TestPlugin::new("beta");
        failed_plugin.state = PluginState::Failed("something broke".into());
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        registry.register(Box::new(failed_plugin)).unwrap();

        let defs = registry.all_tool_definitions();
        // Only alpha's tool should be exported (beta is in Failed state).
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "plugin:alpha:echo");
    }

    #[test]
    fn test_find_tool_invalid_plugin_id_returns_none() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        // Invalid ID with uppercase — PluginId::new() rejects it, so find_tool returns None.
        assert!(registry.find_tool("plugin:INVALID:echo").is_none());
        // ID with spaces
        assert!(registry.find_tool("plugin:has space:echo").is_none());
    }

    #[test]
    fn test_get_mut() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();

        let id = PluginId::from_static("alpha");
        let plugin = registry.get_mut(&id).unwrap();
        assert_eq!(plugin.id().as_str(), "alpha");
    }

    #[test]
    fn test_debug_impl() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new("alpha")))
            .unwrap();
        let debug = format!("{registry:?}");
        assert!(debug.contains("PluginRegistry"));
        assert!(debug.contains("plugin_count"));
    }
}
