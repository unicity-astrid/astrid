//! Plugin trait and core types.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use astrid_core::{ConnectorDescriptor, HookEvent, InboundMessage};

use crate::context::PluginContext;
use crate::error::PluginResult;
use crate::manifest::PluginManifest;
use crate::tool::PluginTool;

/// Unique, stable, human-readable plugin identifier.
///
/// Plugin IDs are strings like `"my-cool-plugin"` or `"openclaw-git-tools"`.
/// They must be non-empty and contain only lowercase alphanumeric characters
/// and hyphens.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct PluginId(String);

/// Deserialize with validation — rejects malformed IDs (e.g. path traversal
/// payloads in crafted lockfiles).
impl<'de> Deserialize<'de> for PluginId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl PluginId {
    /// Create a new `PluginId`, validating the format.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is empty or contains invalid characters.
    pub fn new(id: impl Into<String>) -> PluginResult<Self> {
        let id = id.into();
        Self::validate(&id)?;
        Ok(Self(id))
    }

    /// Create a `PluginId` without validation (for tests and internal use).
    #[must_use]
    pub fn from_static(id: &str) -> Self {
        Self(id.to_string())
    }

    /// Get the inner string value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Check whether a string is a valid plugin ID without constructing one.
    ///
    /// Unlike [`PluginId::new`], this takes a `&str` and returns a bool,
    /// avoiding the `String` allocation needed for `PluginId` construction.
    #[must_use]
    pub fn is_valid_id(id: &str) -> bool {
        Self::validate(id).is_ok()
    }

    /// Validate that a plugin ID string is well-formed.
    fn validate(id: &str) -> PluginResult<()> {
        if id.is_empty() {
            return Err(crate::error::PluginError::InvalidId(
                "plugin id must not be empty".into(),
            ));
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(crate::error::PluginError::InvalidId(format!(
                "plugin id must contain only lowercase alphanumeric characters and hyphens, got: {id}"
            )));
        }
        if id.starts_with('-') || id.ends_with('-') {
            return Err(crate::error::PluginError::InvalidId(format!(
                "plugin id must not start or end with a hyphen, got: {id}"
            )));
        }
        Ok(())
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for PluginId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The lifecycle state of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin is registered but not yet loaded.
    Unloaded,
    /// Plugin is currently loading.
    Loading,
    /// Plugin is loaded and ready to serve tools.
    Ready,
    /// Plugin failed to load or encountered a fatal error.
    Failed(String),
    /// Plugin is shutting down.
    Unloading,
}

impl std::fmt::Debug for dyn Plugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Plugin")
            .field("id", self.id())
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

/// A loaded plugin that can provide tools to the runtime.
///
/// Implementors handle the plugin lifecycle (load/unload) and expose
/// tools that the agent can invoke.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// The unique identifier for this plugin.
    fn id(&self) -> &PluginId;

    /// The manifest that describes this plugin.
    fn manifest(&self) -> &PluginManifest;

    /// Current lifecycle state.
    fn state(&self) -> PluginState;

    /// Load the plugin, initializing any resources it needs.
    ///
    /// Called once when the plugin is first activated. The plugin should
    /// transition from `Unloaded` → `Loading` → `Ready` (or `Failed`).
    async fn load(&mut self, ctx: &PluginContext) -> PluginResult<()>;

    /// Unload the plugin, releasing resources.
    ///
    /// Called when the plugin is being deactivated or the runtime is
    /// shutting down.
    async fn unload(&mut self) -> PluginResult<()>;

    /// The tools this plugin provides.
    ///
    /// Returns an empty slice if the plugin has no tools or is not loaded.
    /// Tools are `Arc`-wrapped so callers can clone a handle and release
    /// the registry lock before executing (avoids holding a read lock
    /// across potentially slow tool calls).
    fn tools(&self) -> &[Arc<dyn PluginTool>];

    /// The connectors this plugin provides.
    ///
    /// Returns an empty slice by default. Plugins that provide connectors
    /// (e.g. a Telegram bridge plugin) override this to return their
    /// connector descriptors.
    fn connectors(&self) -> &[ConnectorDescriptor] {
        &[]
    }

    /// Send a hook event to the plugin.
    ///
    /// The default implementation does nothing. Plugins that need to react to
    /// hook events (e.g. MCP plugins) should override this method.
    async fn send_hook_event(&self, _event: HookEvent, _data: serde_json::Value) {}

    /// Take the inbound message receiver, if any.
    ///
    /// Returns `Some` exactly once — after `load()` succeeds — then `None` on
    /// every subsequent call (single-subscriber). The gateway calls this after
    /// loading to set up message forwarding to the central inbound channel.
    ///
    /// Default: `None` (plugins without connector capability skip this).
    ///
    /// # Implementation contract
    ///
    /// Implementations **must not block or perform async work** in this method.
    /// It may be called while the caller holds a registry lock. The only correct
    /// implementation is `self.inbound_rx.take()`.
    fn take_inbound_rx(&mut self) -> Option<tokio::sync::mpsc::Receiver<InboundMessage>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_plugin_ids() {
        assert!(PluginId::new("my-plugin").is_ok());
        assert!(PluginId::new("openclaw-git-tools").is_ok());
        assert!(PluginId::new("plugin123").is_ok());
        assert!(PluginId::new("a").is_ok());
    }

    #[test]
    fn test_invalid_plugin_ids() {
        // Empty
        assert!(PluginId::new("").is_err());
        // Uppercase
        assert!(PluginId::new("MyPlugin").is_err());
        // Spaces
        assert!(PluginId::new("my plugin").is_err());
        // Underscores
        assert!(PluginId::new("my_plugin").is_err());
        // Leading hyphen
        assert!(PluginId::new("-plugin").is_err());
        // Trailing hyphen
        assert!(PluginId::new("plugin-").is_err());
        // Special characters
        assert!(PluginId::new("plugin@1").is_err());
    }

    #[test]
    fn test_plugin_id_display() {
        let id = PluginId::new("my-plugin").unwrap();
        assert_eq!(id.to_string(), "my-plugin");
        assert_eq!(id.as_str(), "my-plugin");
    }

    #[test]
    fn test_plugin_id_equality() {
        let a = PluginId::new("test-plugin").unwrap();
        let b = PluginId::new("test-plugin").unwrap();
        let c = PluginId::new("other-plugin").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_plugin_id_serde_round_trip() {
        let id = PluginId::new("my-plugin").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"my-plugin\"");
        let deserialized: PluginId = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, id);
    }

    #[test]
    fn test_plugin_state_variants() {
        let states = vec![
            PluginState::Unloaded,
            PluginState::Loading,
            PluginState::Ready,
            PluginState::Failed("timeout".into()),
            PluginState::Unloading,
        ];
        // Ensure Debug works
        for state in &states {
            let _ = format!("{state:?}");
        }
    }

    #[test]
    fn default_take_inbound_rx_returns_none() {
        struct MinimalPlugin;

        #[async_trait::async_trait]
        impl Plugin for MinimalPlugin {
            fn id(&self) -> &PluginId {
                unimplemented!()
            }
            fn manifest(&self) -> &crate::manifest::PluginManifest {
                unimplemented!()
            }
            fn state(&self) -> PluginState {
                PluginState::Unloaded
            }
            async fn load(
                &mut self,
                _ctx: &crate::context::PluginContext,
            ) -> crate::error::PluginResult<()> {
                Ok(())
            }
            async fn unload(&mut self) -> crate::error::PluginResult<()> {
                Ok(())
            }
            fn tools(&self) -> &[Arc<dyn crate::tool::PluginTool>] {
                &[]
            }
        }

        let mut p = MinimalPlugin;
        assert!(
            p.take_inbound_rx().is_none(),
            "default impl must return None"
        );
    }
}
