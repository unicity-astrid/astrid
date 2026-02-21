//! Plugin registry.
//!
//! Manages the set of loaded plugins and provides tool lookup across
//! all registered plugins.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{debug, info};

use astrid_core::identity::FrontendType;
use astrid_core::{ConnectorCapabilities, ConnectorDescriptor, ConnectorId};

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
    connectors: HashMap<ConnectorId, (PluginId, ConnectorDescriptor)>,
}

impl PluginRegistry {
    /// Create an empty plugin registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            connectors: HashMap::new(),
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

        // Register the plugin's connectors, rolling back on failure.
        let mut registered_ids = Vec::new();
        for descriptor in plugin.connectors() {
            match self.register_connector(&id, descriptor.clone()) {
                Ok(()) => registered_ids.push(descriptor.id),
                Err(e) => {
                    for rollback_id in &registered_ids {
                        self.connectors.remove(rollback_id);
                    }
                    return Err(e);
                },
            }
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

        // Clean up the plugin's connectors.
        self.unregister_plugin_connectors(id);

        info!(plugin_id = %id, "Unregistered plugin");
        Ok(plugin)
    }

    /// Unload and remove all plugins from the registry.
    ///
    /// Calls [`Plugin::unload()`] on each plugin, logging errors without
    /// short-circuiting. Connectors are cleaned up as each plugin is removed.
    pub async fn unload_all(&mut self) {
        let ids: Vec<PluginId> = self.plugins.keys().cloned().collect();
        for id in ids {
            if let Some(mut plugin) = self.plugins.remove(&id) {
                self.unregister_plugin_connectors(&id);
                if let Err(e) = plugin.unload().await {
                    tracing::warn!(plugin_id = %id, error = %e, "Plugin unload error during unload_all");
                }
            }
        }
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

    // -----------------------------------------------------------------
    // Connector management
    // -----------------------------------------------------------------

    /// Look up a connector by its ID.
    #[must_use]
    pub fn get_connector(&self, id: &ConnectorId) -> Option<&ConnectorDescriptor> {
        self.connectors.get(id).map(|(_, desc)| desc)
    }

    /// Register a connector for a plugin.
    ///
    /// The `plugin_id` must refer to a plugin that is either already
    /// registered or is in the process of being registered (i.e. called
    /// from within [`register`](Self::register)). Passing a `plugin_id`
    /// that is never registered leaves orphaned connectors that cannot be
    /// cleaned up via [`unregister`](Self::unregister) — use
    /// [`unregister_plugin_connectors`](Self::unregister_plugin_connectors)
    /// to remove them manually.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::ConnectorAlreadyRegistered`] if a connector
    /// with the same ID is already in the registry.
    pub fn register_connector(
        &mut self,
        plugin_id: &PluginId,
        descriptor: ConnectorDescriptor,
    ) -> PluginResult<()> {
        let connector_id = descriptor.id;
        if self.connectors.contains_key(&connector_id) {
            return Err(PluginError::ConnectorAlreadyRegistered(connector_id));
        }
        debug!(
            plugin_id = %plugin_id,
            connector_id = %connector_id,
            connector_name = %descriptor.name,
            "Registered connector"
        );
        self.connectors
            .insert(connector_id, (plugin_id.clone(), descriptor));
        Ok(())
    }

    /// Unregister a single connector by ID, returning it if it was present.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::ConnectorNotFound`] if no connector with the
    /// given ID exists.
    pub fn unregister_connector(&mut self, id: &ConnectorId) -> PluginResult<ConnectorDescriptor> {
        let (_, descriptor) = self
            .connectors
            .remove(id)
            .ok_or(PluginError::ConnectorNotFound(*id))?;
        debug!(connector_id = %id, "Unregistered connector");
        Ok(descriptor)
    }

    /// Remove all connectors belonging to a plugin.
    pub fn unregister_plugin_connectors(&mut self, plugin_id: &PluginId) {
        self.connectors.retain(|_, (owner, _)| owner != plugin_id);
    }

    /// Find a connector that serves the given platform type.
    ///
    /// If multiple connectors serve the same platform, the choice is
    /// arbitrary (`HashMap` iteration order). Returns `None` if no connector
    /// matches.
    #[must_use]
    pub fn find_connector_by_platform(
        &self,
        platform: &FrontendType,
    ) -> Option<&ConnectorDescriptor> {
        self.connectors
            .values()
            .find(|(_, desc)| &desc.frontend_type == platform)
            .map(|(_, desc)| desc)
    }

    /// Find all connectors whose capabilities satisfy the given predicate.
    ///
    /// The order of results is non-deterministic (`HashMap` iteration order).
    #[must_use]
    pub fn find_connectors_with_capability(
        &self,
        check: impl Fn(&ConnectorCapabilities) -> bool,
    ) -> Vec<&ConnectorDescriptor> {
        self.connectors
            .values()
            .filter(|(_, desc)| check(&desc.capabilities))
            .map(|(_, desc)| desc)
            .collect()
    }

    /// List all registered connector descriptors.
    #[must_use]
    pub fn all_connector_descriptors(&self) -> Vec<&ConnectorDescriptor> {
        self.connectors.values().map(|(_, desc)| desc).collect()
    }

    // -----------------------------------------------------------------
    // Tool lookup
    // -----------------------------------------------------------------

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

        // Only return tools from Ready plugins. This mirrors the filter in
        // all_tool_definitions() and prevents execution of stale tools from
        // plugins that transitioned to Failed/Unloaded between listing and dispatch.
        if !matches!(plugin.state(), crate::plugin::PluginState::Ready) {
            return None;
        }

        let tool = plugin.tools().iter().find(|t| t.name() == tool_name)?;

        debug!(
            qualified_name,
            plugin_id = %plugin_id,
            tool_name,
            "Found plugin tool"
        );
        Some((plugin.as_ref(), Arc::clone(tool)))
    }

    /// Check if a tool name refers to a plugin tool (`plugin:{valid_id}:{tool}`).
    ///
    /// Uses [`PluginId::new`] to validate the ID segment, which prevents
    /// collision with an MCP server named `"plugin"` whose tool name contains
    /// a colon (e.g. `"plugin:some:tool"`).
    #[must_use]
    pub fn is_plugin_tool(name: &str) -> bool {
        if let Some(rest) = name.strip_prefix("plugin:")
            && let Some((id, tool_name)) = rest.split_once(':')
        {
            return !tool_name.is_empty() && PluginId::is_valid_id(id);
        }
        false
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
                // Do not expose internal tools (must not be callable by the LLM).
                if tool.name().starts_with("__astrid_") {
                    continue;
                }
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
            .field("connector_count", &self.connectors.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{PluginContext, PluginToolContext};
    use crate::manifest::{PluginEntryPoint, PluginManifest};
    use crate::plugin::PluginState;
    use astrid_core::connector::{ConnectorCapabilities, ConnectorProfile, ConnectorSource};

    /// A test plugin that provides a single tool and optional connectors.
    struct TestPlugin {
        id: PluginId,
        manifest: PluginManifest,
        state: PluginState,
        tools: Vec<Arc<dyn PluginTool>>,
        connectors: Vec<ConnectorDescriptor>,
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
                    connectors: vec![],
                    config: HashMap::new(),
                },
                id: plugin_id,
                state: PluginState::Ready,
                tools: vec![Arc::new(EchoTool)],
                connectors: vec![],
            }
        }

        fn with_no_tools(id: &str) -> Self {
            let mut p = Self::new(id);
            p.tools.clear();
            p
        }

        fn with_connectors(id: &str, connectors: Vec<ConnectorDescriptor>) -> Self {
            let mut p = Self::new(id);
            p.connectors = connectors;
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
        fn connectors(&self) -> &[ConnectorDescriptor] {
            &self.connectors
        }
    }

    struct EchoTool;

    #[async_trait::async_trait]
    impl PluginTool for EchoTool {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn description(&self) -> &'static str {
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
        ids.sort_unstable();
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
    fn test_find_tool_non_ready_plugin_returns_none() {
        let mut registry = PluginRegistry::new();

        // Register a plugin in Failed state that has tools.
        let mut failed_plugin = TestPlugin::new("broken");
        failed_plugin.state = PluginState::Failed("crashed".into());
        registry.register(Box::new(failed_plugin)).unwrap();

        // Even though the tool exists, find_tool rejects non-Ready plugins.
        assert!(
            registry.find_tool("plugin:broken:echo").is_none(),
            "find_tool should return None for non-Ready plugins"
        );
    }

    #[test]
    fn test_find_tool_unloaded_plugin_returns_none() {
        let mut registry = PluginRegistry::new();

        let mut unloaded_plugin = TestPlugin::new("gone");
        unloaded_plugin.state = PluginState::Unloaded;
        registry.register(Box::new(unloaded_plugin)).unwrap();

        assert!(
            registry.find_tool("plugin:gone:echo").is_none(),
            "find_tool should return None for Unloaded plugins"
        );
    }

    #[test]
    fn test_is_plugin_tool_rejects_empty_tool_name() {
        // "plugin:alpha:" has an empty tool name segment — should be rejected.
        assert!(!PluginRegistry::is_plugin_tool("plugin:alpha:"));
    }

    #[test]
    fn test_find_tool_with_colons_in_tool_name() {
        let mut registry = PluginRegistry::new();

        // Create a plugin with a tool whose name contains colons.
        struct ColonTool;

        #[async_trait::async_trait]
        impl PluginTool for ColonTool {
            fn name(&self) -> &'static str {
                "name:with:colons"
            }
            fn description(&self) -> &'static str {
                "A tool with colons in the name"
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object" })
            }
            async fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: &PluginToolContext,
            ) -> PluginResult<String> {
                Ok("ok".to_string())
            }
        }

        let mut plugin = TestPlugin::new("alpha");
        plugin.tools = vec![Arc::new(ColonTool)];
        registry.register(Box::new(plugin)).unwrap();

        // "plugin:alpha:name:with:colons" → split_once on first colon after "alpha"
        // → plugin_id="alpha", tool_name="name:with:colons"
        let result = registry.find_tool("plugin:alpha:name:with:colons");
        assert!(
            result.is_some(),
            "should find tool even with colons in tool name"
        );
        let (_, tool) = result.unwrap();
        assert_eq!(tool.name(), "name:with:colons");
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

    // -----------------------------------------------------------------
    // Connector tests
    // -----------------------------------------------------------------

    fn make_descriptor(name: &str, platform: FrontendType) -> ConnectorDescriptor {
        ConnectorDescriptor::builder(name, platform)
            .source(ConnectorSource::new_wasm("test-plugin").unwrap())
            .capabilities(ConnectorCapabilities::full())
            .profile(ConnectorProfile::Chat)
            .build()
    }

    #[test]
    fn test_register_plugin_with_connectors() {
        let mut registry = PluginRegistry::new();
        let desc = make_descriptor("discord-bot", FrontendType::Discord);
        let connector_id = desc.id;

        registry
            .register(Box::new(TestPlugin::with_connectors("alpha", vec![desc])))
            .unwrap();

        // Connector should be registered alongside the plugin.
        assert_eq!(registry.all_connector_descriptors().len(), 1);
        assert_eq!(registry.all_connector_descriptors()[0].id, connector_id);
    }

    #[test]
    fn test_unregister_plugin_cleans_up_connectors() {
        let mut registry = PluginRegistry::new();
        let desc = make_descriptor("discord-bot", FrontendType::Discord);

        registry
            .register(Box::new(TestPlugin::with_connectors("alpha", vec![desc])))
            .unwrap();
        assert_eq!(registry.all_connector_descriptors().len(), 1);

        let id = PluginId::from_static("alpha");
        registry.unregister(&id).unwrap();

        // Connectors should be cleaned up.
        assert!(registry.all_connector_descriptors().is_empty());
    }

    #[test]
    fn test_register_connector_duplicate_fails() {
        let mut registry = PluginRegistry::new();
        let desc = make_descriptor("bot", FrontendType::Discord);
        let dup = desc.clone();

        let plugin_id = PluginId::from_static("alpha");
        registry.register_connector(&plugin_id, desc).unwrap();

        let result = registry.register_connector(&plugin_id, dup);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::ConnectorAlreadyRegistered(_)
        ));
    }

    #[test]
    fn test_unregister_connector_by_id() {
        let mut registry = PluginRegistry::new();
        let desc = make_descriptor("bot", FrontendType::Discord);
        let connector_id = desc.id;

        let plugin_id = PluginId::from_static("alpha");
        registry.register_connector(&plugin_id, desc).unwrap();
        assert_eq!(registry.all_connector_descriptors().len(), 1);

        let removed = registry.unregister_connector(&connector_id).unwrap();
        assert_eq!(removed.id, connector_id);
        assert!(registry.all_connector_descriptors().is_empty());
    }

    #[test]
    fn test_unregister_connector_not_found() {
        let mut registry = PluginRegistry::new();
        let missing_id = ConnectorId::new();
        let result = registry.unregister_connector(&missing_id);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::ConnectorNotFound(_)
        ));
    }

    #[test]
    fn test_find_connector_by_platform() {
        let mut registry = PluginRegistry::new();
        let discord_desc = make_descriptor("discord-bot", FrontendType::Discord);
        let cli_desc = make_descriptor("cli-bot", FrontendType::Cli);

        registry
            .register(Box::new(TestPlugin::with_connectors(
                "alpha",
                vec![discord_desc.clone(), cli_desc],
            )))
            .unwrap();

        let found = registry.find_connector_by_platform(&FrontendType::Discord);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "discord-bot");

        let found = registry.find_connector_by_platform(&FrontendType::Cli);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "cli-bot");

        // Platform not registered.
        let found = registry.find_connector_by_platform(&FrontendType::Web);
        assert!(found.is_none());
    }

    #[test]
    fn test_find_connectors_with_capability() {
        let mut registry = PluginRegistry::new();

        // Full-capability connector.
        let full = make_descriptor("full-bot", FrontendType::Discord);
        // Notify-only connector (can_approve == false).
        let notify = ConnectorDescriptor::builder("notifier", FrontendType::Cli)
            .capabilities(ConnectorCapabilities::notify_only())
            .build();

        registry
            .register(Box::new(TestPlugin::with_connectors(
                "alpha",
                vec![full, notify],
            )))
            .unwrap();

        // Find connectors that support approval.
        let approval = registry.find_connectors_with_capability(|c| c.can_approve);
        assert_eq!(approval.len(), 1);
        assert_eq!(approval[0].name, "full-bot");

        // Find connectors that can send.
        let senders = registry.find_connectors_with_capability(|c| c.can_send);
        assert_eq!(senders.len(), 2);
    }

    #[test]
    fn test_all_connector_descriptors() {
        let mut registry = PluginRegistry::new();
        assert!(registry.all_connector_descriptors().is_empty());

        let d1 = make_descriptor("a", FrontendType::Discord);
        let d2 = make_descriptor("b", FrontendType::Cli);

        registry
            .register(Box::new(TestPlugin::with_connectors("alpha", vec![d1, d2])))
            .unwrap();

        assert_eq!(registry.all_connector_descriptors().len(), 2);
    }

    #[test]
    fn test_unregister_plugin_connectors_selective() {
        let mut registry = PluginRegistry::new();

        let d1 = make_descriptor("alpha-bot", FrontendType::Discord);
        let d2 = make_descriptor("beta-bot", FrontendType::Cli);

        registry
            .register(Box::new(TestPlugin::with_connectors("alpha", vec![d1])))
            .unwrap();
        registry
            .register(Box::new(TestPlugin::with_connectors("beta", vec![d2])))
            .unwrap();

        assert_eq!(registry.all_connector_descriptors().len(), 2);

        // Unregister alpha — only alpha's connector should be removed.
        let alpha_id = PluginId::from_static("alpha");
        registry.unregister(&alpha_id).unwrap();

        let remaining = registry.all_connector_descriptors();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "beta-bot");
    }

    #[test]
    fn test_register_plugin_rolls_back_connectors_on_failure() {
        let mut registry = PluginRegistry::new();

        // Pre-register a connector directly so the second one in the plugin
        // will collide with it.
        let collider = make_descriptor("collider", FrontendType::Web);
        let collider_id = collider.id;
        let owner = PluginId::from_static("other");
        registry.register_connector(&owner, collider).unwrap();

        // Build a plugin with two connectors: one unique and one that
        // duplicates the pre-registered collider ID.
        let good = make_descriptor("good-bot", FrontendType::Discord);
        let mut bad = make_descriptor("bad-bot", FrontendType::Cli);
        bad.id = collider_id; // force collision

        let plugin = TestPlugin::with_connectors("alpha", vec![good, bad]);
        let result = registry.register(Box::new(plugin));

        // Registration must fail.
        assert!(result.is_err());

        // The first connector ("good-bot") must have been rolled back.
        // Only the original "collider" should remain.
        assert_eq!(registry.all_connector_descriptors().len(), 1);
        assert_eq!(registry.all_connector_descriptors()[0].name, "collider");

        // The plugin itself must not be registered.
        assert!(registry.get(&PluginId::from_static("alpha")).is_none());
    }

    #[test]
    fn test_get_connector_by_id() {
        let mut registry = PluginRegistry::new();
        let desc = make_descriptor("discord-bot", FrontendType::Discord);
        let connector_id = desc.id;

        registry
            .register(Box::new(TestPlugin::with_connectors("alpha", vec![desc])))
            .unwrap();

        let found = registry.get_connector(&connector_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "discord-bot");

        // Missing ID returns None.
        assert!(registry.get_connector(&ConnectorId::new()).is_none());
    }

    #[test]
    fn test_find_connector_by_platform_multiple_matches() {
        let mut registry = PluginRegistry::new();

        // Two connectors from different plugins, same platform.
        let d1 = make_descriptor("bot-a", FrontendType::Discord);
        let d2 = make_descriptor("bot-b", FrontendType::Discord);

        registry
            .register(Box::new(TestPlugin::with_connectors("alpha", vec![d1])))
            .unwrap();
        registry
            .register(Box::new(TestPlugin::with_connectors("beta", vec![d2])))
            .unwrap();

        // Should return one of the two (non-deterministic, but must not panic).
        let found = registry.find_connector_by_platform(&FrontendType::Discord);
        assert!(found.is_some());
        let name = &found.unwrap().name;
        assert!(name == "bot-a" || name == "bot-b");
    }

    #[tokio::test]
    async fn test_unload_all() {
        let mut registry = PluginRegistry::new();
        let desc = make_descriptor("bot", FrontendType::Discord);

        registry
            .register(Box::new(TestPlugin::with_connectors("alpha", vec![desc])))
            .unwrap();
        registry
            .register(Box::new(TestPlugin::new("beta")))
            .unwrap();
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.all_connector_descriptors().len(), 1);

        registry.unload_all().await;

        assert!(registry.is_empty(), "all plugins should be removed");
        assert!(
            registry.all_connector_descriptors().is_empty(),
            "all connectors should be cleaned up"
        );
    }
}
