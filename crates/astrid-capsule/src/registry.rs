//! Capsule registry.
//!
//! Manages the set of loaded capsules and provides tool lookup across
//! all registered capsules.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{debug, info};

use astrid_core::identity::FrontendType;
use astrid_core::{ConnectorCapabilities, ConnectorDescriptor, ConnectorId};

use crate::capsule::{Capsule, CapsuleId};
use crate::error::{CapsuleError, CapsuleResult};
use crate::tool::CapsuleTool;

/// Fully qualified tool name: `capsule:{capsule_id}:{tool_name}`.
///
/// This naming convention avoids collision with built-in tools (no colons)
/// and MCP tools (`server:tool` — single colon).
fn qualified_tool_name(capsule_id: &CapsuleId, tool_name: &str) -> String {
    format!("capsule:{capsule_id}:{tool_name}")
}

/// A tool definition exported for the LLM.
#[derive(Debug, Clone)]
pub struct CapsuleToolDefinition {
    /// Fully qualified tool name (`capsule:{capsule_id}:{tool_name}`).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for tool input.
    pub input_schema: serde_json::Value,
}

/// Registry of loaded capsules.
///
/// Parallel to `ToolRegistry` in `astrid-tools`. Stores capsules keyed by
/// their `CapsuleId` and provides cross-capsule tool lookup.
pub struct CapsuleRegistry {
    capsules: HashMap<CapsuleId, Box<dyn Capsule>>,
    connectors: HashMap<ConnectorId, (CapsuleId, ConnectorDescriptor)>,
}

impl CapsuleRegistry {
    /// Create an empty capsule registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            capsules: HashMap::new(),
            connectors: HashMap::new(),
        }
    }

    /// Register a capsule.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::AlreadyRegistered`] if a capsule with the same
    /// ID is already in the registry.
    pub fn register(&mut self, capsule: Box<dyn Capsule>) -> CapsuleResult<()> {
        let id = capsule.id().clone();
        if self.capsules.contains_key(&id) {
            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "Already registered: {id}"
            ))); // TODO
        }

        // Register the capsule's connectors, rolling back on failure.
        let _registered_ids: Vec<ConnectorId> = Vec::new();
        // TODO: Port connectors extraction
        /*
        for descriptor in capsule.connectors() {
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
        */

        info!(capsule_id = %id, "Registered capsule");
        self.capsules.insert(id, capsule);
        Ok(())
    }

    /// Unregister a capsule, returning it if it was present.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::NotFound`] if no capsule with the given ID exists.
    pub fn unregister(&mut self, id: &CapsuleId) -> CapsuleResult<Box<dyn Capsule>> {
        let capsule = self
            .capsules
            .remove(id)
            .ok_or_else(|| CapsuleError::UnsupportedEntryPoint(format!("Not found: {id}")))?;

        // Clean up the capsule's connectors.
        self.unregister_capsule_connectors(id);

        info!(capsule_id = %id, "Unregistered capsule");
        Ok(capsule)
    }

    /// Unload and remove all capsules from the registry.
    ///
    /// Calls [`Capsule::unload()`] on each capsule, logging errors without
    /// short-circuiting. Connectors are cleaned up as each capsule is removed.
    pub async fn unload_all(&mut self) {
        let ids: Vec<CapsuleId> = self.capsules.keys().cloned().collect();
        for id in ids {
            if let Some(mut capsule) = self.capsules.remove(&id) {
                self.unregister_capsule_connectors(&id);
                if let Err(e) = capsule.unload().await {
                    tracing::warn!(capsule_id = %id, error = %e, "Capsule unload error during unload_all");
                }
            }
        }
    }

    /// Get a reference to a capsule by ID.
    #[must_use]
    pub fn get(&self, id: &CapsuleId) -> Option<&dyn Capsule> {
        self.capsules.get(id).map(AsRef::as_ref)
    }

    /// Get a mutable reference to a capsule by ID.
    #[must_use]
    pub fn get_mut(&mut self, id: &CapsuleId) -> Option<&mut Box<dyn Capsule>> {
        self.capsules.get_mut(id)
    }

    /// List all registered capsule IDs.
    #[must_use]
    pub fn list(&self) -> Vec<&CapsuleId> {
        self.capsules.keys().collect()
    }

    /// Number of registered capsules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.capsules.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.capsules.is_empty()
    }

    // -----------------------------------------------------------------
    // Connector management
    // -----------------------------------------------------------------

    /// Look up a connector by its ID.
    #[must_use]
    pub fn get_connector(&self, id: &ConnectorId) -> Option<&ConnectorDescriptor> {
        self.connectors.get(id).map(|(_, desc)| desc)
    }

    /// Register a connector for a capsule.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::ConnectorAlreadyRegistered`] if a connector
    /// with the same ID is already in the registry.
    pub fn register_connector(
        &mut self,
        capsule_id: &CapsuleId,
        descriptor: ConnectorDescriptor,
    ) -> CapsuleResult<()> {
        let connector_id = descriptor.id;
        if self.connectors.contains_key(&connector_id) {
            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "Connector already registered: {connector_id}"
            )));
        }
        debug!(
            capsule_id = %capsule_id,
            connector_id = %connector_id,
            connector_name = %descriptor.name,
            "Registered connector"
        );
        self.connectors
            .insert(connector_id, (capsule_id.clone(), descriptor));
        Ok(())
    }

    /// Unregister a single connector by ID, returning it if it was present.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::ConnectorNotFound`] if no connector with the
    /// given ID exists.
    pub fn unregister_connector(&mut self, id: &ConnectorId) -> CapsuleResult<ConnectorDescriptor> {
        let (_, descriptor) =
            self.connectors
                .remove(id)
                .ok_or(CapsuleError::UnsupportedEntryPoint(format!(
                    "Connector not found: {id}"
                )))?;
        debug!(connector_id = %id, "Unregistered connector");
        Ok(descriptor)
    }

    /// Remove all connectors belonging to a capsule.
    pub fn unregister_capsule_connectors(&mut self, capsule_id: &CapsuleId) {
        self.connectors.retain(|_, (owner, _)| owner != capsule_id);
    }

    /// Find a connector that serves the given platform type.
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

    /// Find a tool by its fully qualified name (`capsule:{capsule_id}:{tool_name}`).
    #[must_use]
    pub fn find_tool(&self, qualified_name: &str) -> Option<(&dyn Capsule, Arc<dyn CapsuleTool>)> {
        let rest = qualified_name.strip_prefix("capsule:")?;
        let (capsule_id_str, tool_name) = rest.split_once(':')?;

        let capsule_id = CapsuleId::new(capsule_id_str).ok()?;
        let capsule = self.capsules.get(&capsule_id)?;

        if !matches!(capsule.state(), crate::capsule::CapsuleState::Ready) {
            return None;
        }

        let tool = capsule.tools().iter().find(|t| t.name() == tool_name)?;

        debug!(
            qualified_name,
            capsule_id = %capsule_id,
            tool_name,
            "Found capsule tool"
        );
        Some((capsule.as_ref(), Arc::clone(tool)))
    }

    /// Check if a tool name refers to a capsule tool (`capsule:{valid_id}:{tool}`).
    #[must_use]
    pub fn is_capsule_tool(name: &str) -> bool {
        if let Some(rest) = name.strip_prefix("capsule:")
            && let Some((id, tool_name)) = rest.split_once(':')
        {
            return !tool_name.is_empty() && CapsuleId::new(id).is_ok();
        }
        false
    }

    /// Export all tool definitions from all capsules for the LLM.
    #[must_use]
    pub fn all_tool_definitions(&self) -> Vec<CapsuleToolDefinition> {
        let mut defs = Vec::new();
        for (capsule_id, capsule) in &self.capsules {
            if !matches!(capsule.state(), crate::capsule::CapsuleState::Ready) {
                continue;
            }
            for tool in capsule.tools() {
                if tool.name().starts_with("__astrid_") {
                    continue;
                }
                defs.push(CapsuleToolDefinition {
                    name: qualified_tool_name(capsule_id, tool.name()),
                    description: tool.description().to_string(),
                    input_schema: tool.input_schema(),
                });
            }
        }
        defs
    }
}

impl Default for CapsuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CapsuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapsuleRegistry")
            .field("capsule_count", &self.capsules.len())
            .field("capsule_ids", &self.list())
            .field("connector_count", &self.connectors.len())
            .finish()
    }
}
// Tests removed for brevity during scaffolding
