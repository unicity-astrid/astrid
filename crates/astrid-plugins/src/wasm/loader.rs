//! WASM plugin loader with builder-pattern configuration.
//!
//! [`WasmPluginLoader`] is the factory for creating [`WasmPlugin`] instances
//! with shared configuration (security gate, memory limits, timeouts).

use std::sync::Arc;
use std::time::Duration;

use crate::manifest::PluginManifest;
use crate::security::PluginSecurityGate;
use crate::wasm::plugin::{WasmPlugin, WasmPluginConfig};

/// Default maximum WASM linear memory: 64 MB.
const DEFAULT_MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024;

/// Default maximum execution time per call: 30 seconds.
const DEFAULT_MAX_EXECUTION_TIME: Duration = Duration::from_secs(30);

/// Factory for creating [`WasmPlugin`] instances with shared configuration.
///
/// # Example
///
/// ```rust,no_run
/// use astrid_plugins::wasm::WasmPluginLoader;
/// use std::time::Duration;
///
/// let loader = WasmPluginLoader::new()
///     .with_memory_limit(32 * 1024 * 1024) // 32 MB
///     .with_timeout(Duration::from_secs(10));
/// ```
pub struct WasmPluginLoader {
    security: Option<Arc<dyn PluginSecurityGate>>,
    max_memory_bytes: u64,
    max_execution_time: Duration,
    require_hash: bool,
}

impl std::fmt::Debug for WasmPluginLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmPluginLoader")
            .field("has_security", &self.security.is_some())
            .field("max_memory_bytes", &self.max_memory_bytes)
            .field("max_execution_time", &self.max_execution_time)
            .field("require_hash", &self.require_hash)
            .finish()
    }
}

impl Default for WasmPluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmPluginLoader {
    /// Create a new loader with default settings (64 MB memory, 30s timeout).
    #[must_use]
    pub fn new() -> Self {
        Self {
            security: None,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_execution_time: DEFAULT_MAX_EXECUTION_TIME,
            require_hash: false,
        }
    }

    /// Set the security gate for authorizing host function calls.
    #[must_use]
    pub fn with_security(mut self, gate: Arc<dyn PluginSecurityGate>) -> Self {
        self.security = Some(gate);
        self
    }

    /// Set the maximum WASM linear memory in bytes.
    #[must_use]
    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.max_memory_bytes = bytes;
        self
    }

    /// Set the maximum execution time per WASM call.
    #[must_use]
    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.max_execution_time = duration;
        self
    }

    /// Require WASM modules to have a hash in their manifest.
    ///
    /// When enabled, plugins without a `hash` field in their manifest
    /// entry point will fail to load. Recommended for production.
    #[must_use]
    pub fn with_require_hash(mut self, require: bool) -> Self {
        self.require_hash = require;
        self
    }

    /// Create an unloaded [`WasmPlugin`] from a manifest.
    ///
    /// The plugin must be loaded via [`Plugin::load()`](crate::Plugin::load)
    /// before it can serve tools.
    #[must_use]
    pub fn create_plugin(&self, manifest: PluginManifest) -> WasmPlugin {
        let config = WasmPluginConfig {
            security: self.security.clone(),
            max_memory_bytes: self.max_memory_bytes,
            max_execution_time: self.max_execution_time,
            require_hash: self.require_hash,
        };
        WasmPlugin::new(manifest, config)
    }

    /// Get the configured memory limit.
    #[must_use]
    pub fn max_memory_bytes(&self) -> u64 {
        self.max_memory_bytes
    }

    /// Get the configured execution timeout.
    #[must_use]
    pub fn max_execution_time(&self) -> Duration {
        self.max_execution_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let loader = WasmPluginLoader::new();
        assert_eq!(loader.max_memory_bytes(), 64 * 1024 * 1024);
        assert_eq!(loader.max_execution_time(), Duration::from_secs(30));
    }

    #[test]
    fn custom_config() {
        let loader = WasmPluginLoader::new()
            .with_memory_limit(32 * 1024 * 1024)
            .with_timeout(Duration::from_secs(10));
        assert_eq!(loader.max_memory_bytes(), 32 * 1024 * 1024);
        assert_eq!(loader.max_execution_time(), Duration::from_secs(10));
    }

    #[test]
    fn create_plugin_returns_unloaded() {
        use crate::Plugin;
        use crate::manifest::{PluginEntryPoint, PluginManifest};
        use crate::plugin::PluginState;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let loader = WasmPluginLoader::new();
        let manifest = PluginManifest {
            id: crate::PluginId::from_static("test"),
            name: "Test".into(),
            version: "0.1.0".into(),
            description: None,
            author: None,
            entry_point: PluginEntryPoint::Wasm {
                path: PathBuf::from("test.wasm"),
                hash: None,
            },
            capabilities: vec![],
            connectors: vec![],
            config: HashMap::new(),
        };
        let plugin = loader.create_plugin(manifest);
        assert_eq!(plugin.state(), PluginState::Unloaded);
    }
}
