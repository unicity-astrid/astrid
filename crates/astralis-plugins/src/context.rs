//! Plugin context types.
//!
//! Provides the execution context for plugin lifecycle and tool invocations.
//! Combines relevant fields from `HookContext` (session/user), `ToolContext`
//! (workspace), and `ScopedKvStore` (plugin-scoped storage).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use astralis_core::SessionId;
use astralis_storage::kv::ScopedKvStore;
use uuid::Uuid;

use crate::PluginId;

/// Context provided to a plugin during lifecycle operations (load/unload).
///
/// Contains the information a plugin needs to initialize itself.
#[derive(Debug, Clone)]
pub struct PluginContext {
    /// The workspace root directory.
    pub workspace_root: PathBuf,
    /// Pre-scoped KV store for this plugin (`plugin:{plugin_id}` namespace).
    pub kv: ScopedKvStore,
    /// Plugin configuration from the manifest.
    pub config: HashMap<String, serde_json::Value>,
}

impl PluginContext {
    /// Create a new plugin context.
    #[must_use]
    pub fn new(
        workspace_root: PathBuf,
        kv: ScopedKvStore,
        config: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            workspace_root,
            kv,
            config,
        }
    }

    /// Create a plugin context with a `MemoryKvStore` for testing.
    ///
    /// # Errors
    ///
    /// Returns an error if the scoped KV store cannot be created.
    pub fn for_testing(
        plugin_id: &PluginId,
        workspace_root: PathBuf,
    ) -> crate::error::PluginResult<Self> {
        let store = Arc::new(astralis_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, format!("plugin:{plugin_id}"))?;
        Ok(Self {
            workspace_root,
            kv,
            config: HashMap::new(),
        })
    }
}

/// Context provided to a plugin tool during execution.
///
/// Combines:
/// - Plugin identity (`plugin_id`)
/// - Workspace root (from `ToolContext`)
/// - Scoped KV store (pre-bound to `plugin:{plugin_id}`)
/// - Plugin config
/// - Session/user info (from `HookContext`)
#[derive(Debug, Clone)]
pub struct PluginToolContext {
    /// The plugin this tool belongs to.
    pub plugin_id: PluginId,
    /// The workspace root directory.
    pub workspace_root: PathBuf,
    /// Pre-scoped KV store (`plugin:{plugin_id}` namespace).
    pub kv: ScopedKvStore,
    /// Plugin configuration from the manifest.
    pub config: HashMap<String, serde_json::Value>,
    /// Current session ID, if available.
    pub session_id: Option<SessionId>,
    /// Current user ID, if available.
    pub user_id: Option<Uuid>,
}

impl PluginToolContext {
    /// Create a new plugin tool context.
    #[must_use]
    pub fn new(plugin_id: PluginId, workspace_root: PathBuf, kv: ScopedKvStore) -> Self {
        Self {
            plugin_id,
            workspace_root,
            kv,
            config: HashMap::new(),
            session_id: None,
            user_id: None,
        }
    }

    /// Set the plugin configuration.
    #[must_use]
    pub fn with_config(mut self, config: HashMap<String, serde_json::Value>) -> Self {
        self.config = config;
        self
    }

    /// Set the session ID.
    #[must_use]
    pub fn with_session(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Set the user ID.
    #[must_use]
    pub fn with_user(mut self, user_id: Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }
}
