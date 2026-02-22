//! Shared state for Extism host functions.
//!
//! [`HostState`] is wrapped in [`extism::UserData`] and shared across all
//! host function invocations for a single plugin instance.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use astrid_core::connector::{ConnectorDescriptor, InboundMessage, MAX_CONNECTORS_PER_PLUGIN};
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
    /// System Event Bus for IPC publish/subscribe.
    pub event_bus: astrid_events::EventBus,
    /// Rate limiter for IPC message publishing.
    pub ipc_limiter: astrid_events::ipc::IpcRateLimiter,
    /// Active event bus subscriptions for IPC events.
    pub subscriptions: HashMap<u64, astrid_events::EventReceiver>,
    /// Counter for issuing subscription handle IDs.
    pub next_subscription_id: u64,
    /// Plugin configuration from the manifest.
    pub config: HashMap<String, serde_json::Value>,
    /// Optional security gate for gated operations (HTTP, file I/O).
    pub security: Option<Arc<dyn PluginSecurityGate>>,
    /// Tokio runtime handle for bridging async operations in sync host functions.
    pub runtime_handle: tokio::runtime::Handle,
    /// Whether the plugin manifest declares `PluginCapability::Connector`.
    ///
    /// Used to gate `astrid_register_connector` — only connector plugins
    /// are allowed to register connectors.
    pub has_connector_capability: bool,
    /// Sender for inbound messages from connector plugins.
    ///
    /// Set during plugin loading when the manifest declares
    /// [`PluginCapability::Connector`](crate::PluginCapability). Feeds into
    /// the gateway's inbound router.
    pub inbound_tx: Option<mpsc::Sender<InboundMessage>>,
    /// Connectors registered by the WASM guest via `astrid_register_connector`.
    pub registered_connectors: Vec<ConnectorDescriptor>,
}

impl HostState {
    /// Register a connector descriptor (called from the host function).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The per-plugin connector limit ([`MAX_CONNECTORS_PER_PLUGIN`]) has been reached.
    /// - A connector with the same name and platform already exists.
    pub fn register_connector(
        &mut self,
        descriptor: ConnectorDescriptor,
    ) -> Result<(), &'static str> {
        if self.registered_connectors.len() >= MAX_CONNECTORS_PER_PLUGIN {
            return Err("connector registration limit reached");
        }
        // Reject duplicate name+platform combinations
        let duplicate = self
            .registered_connectors
            .iter()
            .any(|c| c.name == descriptor.name && c.frontend_type == descriptor.frontend_type);
        if duplicate {
            return Err("duplicate connector name and platform");
        }
        self.registered_connectors.push(descriptor);
        Ok(())
    }

    /// Return the registered connectors.
    #[must_use]
    pub fn connectors(&self) -> &[ConnectorDescriptor] {
        &self.registered_connectors
    }

    /// Set the inbound message sender.
    pub fn set_inbound_tx(&mut self, tx: mpsc::Sender<InboundMessage>) {
        self.inbound_tx = Some(tx);
    }
}

impl std::fmt::Debug for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("plugin_id", &self.plugin_id)
            .field("workspace_root", &self.workspace_root)
            .field("has_security", &self.security.is_some())
            .field("has_connector_capability", &self.has_connector_capability)
            .field("has_inbound_tx", &self.inbound_tx.is_some())
            .field("registered_connectors", &self.registered_connectors.len())
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
            event_bus: astrid_events::EventBus::new(),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            security: None,
            runtime_handle: rt.handle().clone(),
            has_connector_capability: false,
            inbound_tx: None,
            registered_connectors: Vec::new(),
        };

        let debug = format!("{state:?}");
        assert!(debug.contains("test"));
        assert!(debug.contains("has_security"));
        assert!(debug.contains("has_inbound_tx"));
        assert!(debug.contains("registered_connectors"));
    }

    #[test]
    fn register_connector_accumulates() {
        use astrid_core::connector::{ConnectorCapabilities, ConnectorProfile, ConnectorSource};
        use astrid_core::identity::FrontendType;

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "plugin:test").unwrap();

        let mut state = HostState {
            plugin_id: PluginId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            kv,
            event_bus: astrid_events::EventBus::new(),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            security: None,
            runtime_handle: rt.handle().clone(),
            has_connector_capability: true,
            inbound_tx: None,
            registered_connectors: Vec::new(),
        };

        assert!(state.connectors().is_empty());

        let desc = ConnectorDescriptor::builder("test-conn", FrontendType::Discord)
            .source(ConnectorSource::Wasm {
                plugin_id: "test".into(),
            })
            .capabilities(ConnectorCapabilities::receive_only())
            .profile(ConnectorProfile::Chat)
            .build();
        state.register_connector(desc).unwrap();

        assert_eq!(state.connectors().len(), 1);
        assert_eq!(state.connectors()[0].name, "test-conn");
    }

    #[test]
    fn set_inbound_tx_stores_sender() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "plugin:test").unwrap();

        let mut state = HostState {
            plugin_id: PluginId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            kv,
            event_bus: astrid_events::EventBus::new(),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            security: None,
            runtime_handle: rt.handle().clone(),
            has_connector_capability: false,
            inbound_tx: None,
            registered_connectors: Vec::new(),
        };

        assert!(state.inbound_tx.is_none());

        let (tx, _rx) = mpsc::channel(256);
        state.set_inbound_tx(tx);

        assert!(state.inbound_tx.is_some());
    }

    #[test]
    fn register_connector_rejects_at_limit() {
        use astrid_core::connector::{ConnectorCapabilities, ConnectorProfile, ConnectorSource};
        use astrid_core::identity::FrontendType;

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "plugin:test").unwrap();

        let mut state = HostState {
            plugin_id: PluginId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            kv,
            event_bus: astrid_events::EventBus::new(),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            security: None,
            runtime_handle: rt.handle().clone(),
            has_connector_capability: true,
            inbound_tx: None,
            registered_connectors: Vec::new(),
        };

        // Fill to the limit
        for i in 0..MAX_CONNECTORS_PER_PLUGIN {
            let desc = ConnectorDescriptor::builder(format!("conn-{i}"), FrontendType::Discord)
                .source(ConnectorSource::Wasm {
                    plugin_id: "test".into(),
                })
                .capabilities(ConnectorCapabilities::receive_only())
                .profile(ConnectorProfile::Chat)
                .build();
            assert!(state.register_connector(desc).is_ok());
        }

        assert_eq!(state.connectors().len(), MAX_CONNECTORS_PER_PLUGIN);

        // One more should fail
        let extra = ConnectorDescriptor::builder("over-limit", FrontendType::Discord)
            .source(ConnectorSource::Wasm {
                plugin_id: "test".into(),
            })
            .capabilities(ConnectorCapabilities::receive_only())
            .profile(ConnectorProfile::Chat)
            .build();
        assert!(state.register_connector(extra).is_err());
        assert_eq!(state.connectors().len(), MAX_CONNECTORS_PER_PLUGIN);
    }

    #[test]
    fn register_connector_rejects_duplicate_name_and_platform() {
        use astrid_core::connector::{ConnectorCapabilities, ConnectorProfile, ConnectorSource};
        use astrid_core::identity::FrontendType;

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(store, "plugin:test").unwrap();

        let mut state = HostState {
            plugin_id: PluginId::from_static("test"),
            workspace_root: PathBuf::from("/tmp"),
            kv,
            event_bus: astrid_events::EventBus::new(),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            security: None,
            runtime_handle: rt.handle().clone(),
            has_connector_capability: true,
            inbound_tx: None,
            registered_connectors: Vec::new(),
        };

        let desc1 = ConnectorDescriptor::builder("my-conn", FrontendType::Discord)
            .source(ConnectorSource::Wasm {
                plugin_id: "test".into(),
            })
            .capabilities(ConnectorCapabilities::receive_only())
            .profile(ConnectorProfile::Chat)
            .build();
        assert!(state.register_connector(desc1).is_ok());

        // Same name + same platform → rejected
        let desc2 = ConnectorDescriptor::builder("my-conn", FrontendType::Discord)
            .source(ConnectorSource::Wasm {
                plugin_id: "test".into(),
            })
            .capabilities(ConnectorCapabilities::receive_only())
            .profile(ConnectorProfile::Chat)
            .build();
        let err = state.register_connector(desc2).unwrap_err();
        assert!(err.contains("duplicate"), "expected duplicate error: {err}");

        // Same name + different platform → allowed
        let desc3 = ConnectorDescriptor::builder("my-conn", FrontendType::Telegram)
            .source(ConnectorSource::Wasm {
                plugin_id: "test".into(),
            })
            .capabilities(ConnectorCapabilities::receive_only())
            .profile(ConnectorProfile::Chat)
            .build();
        assert!(state.register_connector(desc3).is_ok());
        assert_eq!(state.connectors().len(), 2);
    }
}
