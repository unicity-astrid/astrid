//! Plugin trait and core types.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
    fn tools(&self) -> &[Box<dyn PluginTool>];
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
}
