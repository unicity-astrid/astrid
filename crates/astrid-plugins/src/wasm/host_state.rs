//! Shared state for Extism host functions.
//!
//! [`HostState`] is wrapped in [`extism::UserData`] and shared across all
//! host function invocations for a single plugin instance.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use astrid_storage::kv::ScopedKvStore;

use crate::PluginId;
use crate::security::PluginSecurityGate;

/// Shared state accessible to all host functions via `UserData<HostState>`.
pub struct HostState {
    /// The plugin this state belongs to.
    pub plugin_id: PluginId,
    /// Workspace root directory (file operations are confined here).
    pub workspace_root: PathBuf,
    /// Plugin-scoped KV store (`plugin:{plugin_id}` namespace).
    pub kv: ScopedKvStore,
    /// Plugin configuration from the manifest.
    pub config: HashMap<String, serde_json::Value>,
    /// Optional security gate for gated operations (HTTP, file I/O).
    pub security: Option<Arc<dyn PluginSecurityGate>>,
    /// Tokio runtime handle for bridging async operations in sync host functions.
    pub runtime_handle: tokio::runtime::Handle,
}

impl std::fmt::Debug for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("plugin_id", &self.plugin_id)
            .field("workspace_root", &self.workspace_root)
            .field("has_security", &self.security.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_state_debug_format() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "plugin:test").unwrap();

        let state = HostState {
            plugin_id: PluginId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            kv,
            config: HashMap::new(),
            security: None,
            runtime_handle: rt.handle().clone(),
        };

        let debug = format!("{state:?}");
        assert!(debug.contains("test"));
        assert!(debug.contains("has_security"));
    }
}
