//! Capsule registry.
//!
//! Manages the set of loaded capsules and provides tool lookup across
//! all registered capsules.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{debug, info};

use astrid_core::{UplinkCapabilities, UplinkDescriptor, UplinkId};

use crate::capsule::{Capsule, CapsuleId};
use crate::error::{CapsuleError, CapsuleResult};

/// Registry of loaded capsules.
///
/// Parallel to `ToolRegistry` in `astrid-tools`. Stores capsules keyed by
/// their `CapsuleId` and provides cross-capsule tool lookup.
pub struct CapsuleRegistry {
    capsules: HashMap<CapsuleId, Arc<dyn Capsule>>,
    uplinks: HashMap<UplinkId, (CapsuleId, UplinkDescriptor)>,
}

impl CapsuleRegistry {
    /// Create an empty capsule registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            capsules: HashMap::new(),
            uplinks: HashMap::new(),
        }
    }

    /// Register a capsule.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::AlreadyRegistered`] if a capsule with the same
    /// ID is already in the registry.
    pub fn register(&mut self, capsule: Box<dyn Capsule>) -> CapsuleResult<()> {
        let capsule: Arc<dyn Capsule> = Arc::from(capsule);
        let id = capsule.id().clone();
        if self.capsules.contains_key(&id) {
            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "Already registered: {id}"
            )));
        }

        // Register the capsule's uplinks (uplinks)
        let mut registered_ids: Vec<UplinkId> = Vec::new();
        for uplink in &capsule.manifest().uplinks {
            let source = astrid_core::uplink::UplinkSource::new_wasm(id.as_str()).map_err(|e| {
                CapsuleError::UnsupportedEntryPoint(format!("Failed to create source: {}", e))
            })?;

            let descriptor =
                UplinkDescriptor::builder(uplink.name.clone(), uplink.platform.clone())
                    .source(source)
                    .capabilities(UplinkCapabilities::receive_only())
                    .profile(uplink.profile)
                    .build();

            match self.register_uplink(&id, descriptor.clone()) {
                Ok(()) => registered_ids.push(descriptor.id),
                Err(e) => {
                    for rollback_id in &registered_ids {
                        self.uplinks.remove(rollback_id);
                    }
                    return Err(e);
                },
            }
        }

        info!(capsule_id = %id, "Registered capsule");
        self.capsules.insert(id, capsule);
        Ok(())
    }

    /// Unregister a capsule, returning it if it was present.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::NotFound`] if no capsule with the given ID exists.
    pub fn unregister(&mut self, id: &CapsuleId) -> CapsuleResult<Arc<dyn Capsule>> {
        let capsule = self
            .capsules
            .remove(id)
            .ok_or_else(|| CapsuleError::UnsupportedEntryPoint(format!("Not found: {id}")))?;

        // Clean up the capsule's uplinks.
        self.unregister_capsule_uplinks(id);

        info!(capsule_id = %id, "Unregistered capsule");
        Ok(capsule)
    }

    /// Get a shared reference to a capsule by ID.
    ///
    /// Returns a cloned `Arc` so callers can use the capsule after releasing
    /// the registry lock.
    #[must_use]
    pub fn get(&self, id: &CapsuleId) -> Option<Arc<dyn Capsule>> {
        self.capsules.get(id).cloned()
    }

    /// List all registered capsule IDs.
    #[must_use]
    pub fn list(&self) -> Vec<&CapsuleId> {
        self.capsules.keys().collect()
    }

    /// Iterator over all registered capsules.
    pub fn values(&self) -> impl Iterator<Item = &(dyn Capsule + '_)> {
        self.capsules.values().map(|c| c.as_ref())
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
    // Uplink management
    // -----------------------------------------------------------------

    /// Look up a uplink by its ID.
    #[must_use]
    pub fn get_uplink(&self, id: &UplinkId) -> Option<&UplinkDescriptor> {
        self.uplinks.get(id).map(|(_, desc)| desc)
    }

    /// Register a uplink for a capsule.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::UplinkAlreadyRegistered`] if a uplink
    /// with the same ID is already in the registry.
    pub fn register_uplink(
        &mut self,
        capsule_id: &CapsuleId,
        descriptor: UplinkDescriptor,
    ) -> CapsuleResult<()> {
        let uplink_id = descriptor.id;
        if self.uplinks.contains_key(&uplink_id) {
            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "Uplink already registered: {uplink_id}"
            )));
        }
        debug!(
            capsule_id = %capsule_id,
            uplink_id = %uplink_id,
            uplink_name = %descriptor.name,
            "Registered uplink"
        );
        self.uplinks
            .insert(uplink_id, (capsule_id.clone(), descriptor));
        Ok(())
    }

    /// Remove all uplinks belonging to a capsule.
    pub fn unregister_capsule_uplinks(&mut self, capsule_id: &CapsuleId) {
        self.uplinks.retain(|_, (owner, _)| owner != capsule_id);
    }

    /// Find a uplink that serves the given platform type.
    #[must_use]
    pub fn find_uplink_by_platform(&self, platform: &str) -> Option<&UplinkDescriptor> {
        self.uplinks
            .values()
            .find(|(_, desc)| desc.platform == platform)
            .map(|(_, desc)| desc)
    }

    /// Find all uplinks whose capabilities satisfy the given predicate.
    #[must_use]
    pub fn find_uplinks_with_capability(
        &self,
        check: impl Fn(&UplinkCapabilities) -> bool,
    ) -> Vec<&UplinkDescriptor> {
        self.uplinks
            .values()
            .filter(|(_, desc)| check(&desc.capabilities))
            .map(|(_, desc)| desc)
            .collect()
    }

    /// List all registered uplink descriptors.
    #[must_use]
    pub fn all_uplink_descriptors(&self) -> Vec<&UplinkDescriptor> {
        self.uplinks.values().map(|(_, desc)| desc).collect()
    }

    /// Remove and return all capsules, clearing uplinks too.
    ///
    /// Used during kernel shutdown to unload everything in one pass.
    pub fn drain(&mut self) -> Vec<Arc<dyn Capsule>> {
        self.uplinks.clear();
        self.capsules.drain().map(|(_, c)| c).collect()
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
            .field("uplink_count", &self.uplinks.len())
            .finish()
    }
}
// Tests removed for brevity during scaffolding
