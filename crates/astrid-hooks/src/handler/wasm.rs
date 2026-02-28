//! WASM hook handler powered by Extism.
//!
//! Loads a WASM module and calls its `run-hook` export, passing a serialized
//! [`PluginContext`](astrid_core::plugin_abi::PluginContext) and interpreting
//! the returned [`PluginResult`](astrid_core::plugin_abi::PluginResult).
//!
//! Host functions are shared with the plugin system via
//! [`astrid_plugins::wasm::host_functions`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astrid_capsule::capsule::CapsuleId;
use astrid_capsule::engine::wasm::host::register_host_functions;
use astrid_capsule::engine::wasm::host_state::HostState;
use astrid_core::plugin_abi;
use astrid_storage::kv::ScopedKvStore;
use extism::{Manifest, PluginBuilder, UserData, Wasm};
use tracing::{debug, warn};

use super::{HandlerError, HandlerResult};
use crate::hook::HookHandler;
use crate::result::{HookContext, HookExecutionResult, HookResult};

/// Handler for WASM modules.
///
/// Lazily loads the WASM module on first invocation and caches the Extism
/// plugin instance for subsequent calls.
pub struct WasmHandler {
    /// Cached Extism plugin (lazy-loaded).
    cached_plugin: Mutex<HashMap<String, Arc<Mutex<extism::Plugin>>>>,
    /// Configuration for WASM execution.
    config: WasmConfig,
    /// KV store for hook state (scoped to `hook:wasm`).
    kv: Option<ScopedKvStore>,
    /// Workspace root for file operations.
    workspace_root: PathBuf,
}

impl WasmHandler {
    /// Create a new WASM handler.
    #[must_use]
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            cached_plugin: Mutex::new(HashMap::new()),
            config: WasmConfig::default(),
            kv: None,
            workspace_root,
        }
    }

    /// Set the KV store for hook state persistence.
    #[must_use]
    pub fn with_kv(mut self, kv: ScopedKvStore) -> Self {
        self.kv = Some(kv);
        self
    }

    /// Set the WASM execution configuration.
    #[must_use]
    pub fn with_config(mut self, config: WasmConfig) -> Self {
        self.config = config;
        self
    }

    /// Execute a WASM handler.
    ///
    /// Loads the WASM module (or uses the cached instance), then calls the
    /// specified function with a serialized `PluginContext`.
    ///
    /// # Errors
    ///
    /// Returns an error if the module fails to load or the function call fails.
    #[allow(clippy::unused_async)]
    pub async fn execute(
        &self,
        handler: &HookHandler,
        context: &HookContext,
        _timeout: Duration,
    ) -> HandlerResult<HookExecutionResult> {
        let HookHandler::Wasm {
            module_path,
            function,
        } = handler
        else {
            return Err(HandlerError::InvalidConfiguration(
                "expected Wasm handler".to_string(),
            ));
        };

        debug!(module_path = %module_path, function = %function, "executing WASM hook handler");

        // Get or create cached plugin instance
        let plugin = self
            .get_or_load_plugin(module_path)
            .map_err(|e| HandlerError::WasmFailed(format!("failed to load WASM module: {e}")))?;

        // Build PluginContext from HookContext
        let plugin_context = plugin_abi::PluginContext {
            event: context.event.to_string(),
            session_id: context
                .session_id
                .map_or_else(String::new, |id| id.to_string()),
            user_id: context.user_id.map(|id| id.to_string()),
            data: if context.data.is_empty() {
                None
            } else {
                serde_json::to_string(&context.data).ok()
            },
        };

        let input_json = serde_json::to_string(&plugin_context)
            .map_err(|e| HandlerError::WasmFailed(format!("failed to serialize context: {e}")))?;

        // Call the WASM function
        let result = tokio::task::block_in_place(|| {
            let mut plugin_guard = plugin
                .lock()
                .map_err(|e| HandlerError::WasmFailed(format!("plugin lock poisoned: {e}")))?;
            plugin_guard
                .call::<&str, String>(function, &input_json)
                .map_err(|e| HandlerError::WasmFailed(format!("{function} call failed: {e}")))
        })?;

        // Parse PluginResult
        let plugin_result: plugin_abi::PluginResult = serde_json::from_str(&result)
            .map_err(|e| HandlerError::ParseError(format!("failed to parse PluginResult: {e}")))?;

        // Map PluginResult.action to HookResult
        let hook_result = map_plugin_result_to_hook_result(&plugin_result);

        Ok(HookExecutionResult::Success {
            result: hook_result,
            stdout: None,
        })
    }

    /// Check if the WASM runtime is available.
    #[must_use]
    pub fn is_available() -> bool {
        true
    }

    /// Get a cached plugin or load it from disk.
    fn get_or_load_plugin(
        &self,
        module_path: &str,
    ) -> Result<Arc<Mutex<extism::Plugin>>, HandlerError> {
        let mut cache = self
            .cached_plugin
            .lock()
            .map_err(|e| HandlerError::WasmFailed(format!("cache lock poisoned: {e}")))?;

        if let Some(plugin) = cache.get(module_path) {
            return Ok(Arc::clone(plugin));
        }

        // Load the WASM module
        let wasm_path = PathBuf::from(module_path);
        let resolved = if wasm_path.is_absolute() {
            wasm_path
        } else {
            self.workspace_root.join(&wasm_path)
        };

        let wasm_bytes = std::fs::read(&resolved).map_err(|e| {
            HandlerError::WasmFailed(format!(
                "failed to read WASM module {}: {e}",
                resolved.display()
            ))
        })?;

        // Build host state (hooks get a simple HostState with no security gate)
        let kv = if let Some(kv) = &self.kv {
            kv.clone()
        } else {
            let store = Arc::new(astrid_storage::MemoryKvStore::new());
            ScopedKvStore::new(store, "hook:wasm")
                .map_err(|e| HandlerError::WasmFailed(format!("failed to create KV store: {e}")))?
        };

        let vfs = astrid_vfs::HostVfs::new();
        let root_handle = astrid_capabilities::DirHandle::new();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(vfs.register_dir(root_handle.clone(), self.workspace_root.clone()))
                .expect("Failed to register VFS root dir");
        });

        let host_state = HostState {
            capsule_uuid: uuid::Uuid::new_v4(),
            caller_context: None,
            capsule_id: CapsuleId::from_static("hook-wasm"),
            workspace_root: self.workspace_root.clone(),
            vfs: Arc::new(vfs),
            vfs_root_handle: root_handle,
            upper_dir: None,
            kv,
            event_bus: astrid_events::EventBus::with_capacity(128),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            security: None,
            hook_manager: None,
            runtime_handle: tokio::runtime::Handle::current(),
            has_connector_capability: false,
            inbound_tx: None,
            registered_connectors: Vec::new(),
        };
        let user_data = UserData::new(host_state);

        // Build Extism plugin
        let extism_wasm = Wasm::data(wasm_bytes);
        let mut extism_manifest = Manifest::new([extism_wasm]);
        extism_manifest = extism_manifest.with_timeout(self.config.max_execution_time);
        // WASM pages are 64KB each; cap at u32::MAX pages if the byte limit is very large
        let pages = self.config.max_memory_bytes / (64 * 1024);
        let max_pages = u32::try_from(pages).unwrap_or(u32::MAX);
        extism_manifest = extism_manifest.with_memory_max(max_pages);

        let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
        let builder = register_host_functions(builder, user_data);
        let plugin = builder
            .build()
            .map_err(|e| HandlerError::WasmFailed(format!("failed to build Extism plugin: {e}")))?;

        let plugin_arc = Arc::new(Mutex::new(plugin));
        cache.insert(module_path.to_string(), Arc::clone(&plugin_arc));

        Ok(plugin_arc)
    }
}

impl std::fmt::Debug for WasmHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHandler")
            .field("config", &self.config)
            .field("workspace_root", &self.workspace_root)
            .finish_non_exhaustive()
    }
}

/// Map a `PluginResult` action string to a `HookResult`.
fn map_plugin_result_to_hook_result(result: &plugin_abi::PluginResult) -> HookResult {
    match result.action.as_str() {
        "continue" => HookResult::Continue,
        "block" => {
            let reason = result.data.as_deref().unwrap_or("blocked by WASM hook");
            HookResult::block(reason)
        },
        "ask" => {
            let question = result
                .data
                .as_deref()
                .unwrap_or("WASM hook requests user input");
            HookResult::ask(question)
        },
        "modify" => {
            // Parse modifications from data JSON
            if let Some(data) = &result.data
                && let Ok(modifications) = serde_json::from_str(data)
            {
                return HookResult::ContinueWith { modifications };
            }
            HookResult::Continue
        },
        other => {
            warn!(action = %other, "unknown PluginResult action, treating as continue");
            HookResult::Continue
        },
    }
}

/// Configuration for WASM execution.
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Maximum memory in bytes.
    pub max_memory_bytes: u64,
    /// Maximum execution time.
    pub max_execution_time: Duration,
    /// Enable WASI.
    pub enable_wasi: bool,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_execution_time: Duration::from_secs(30),
            enable_wasi: true,
        }
    }
}

/// WASM module metadata.
#[derive(Debug, Clone)]
pub struct WasmModuleInfo {
    /// Module path.
    pub path: String,
    /// Module hash (for verification).
    pub hash: Option<String>,
    /// Exported functions.
    pub exports: Vec<String>,
    /// Required imports.
    pub imports: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookEvent;

    #[test]
    fn test_wasm_available() {
        assert!(WasmHandler::is_available());
    }

    #[test]
    fn test_wasm_config_default() {
        let config = WasmConfig::default();
        assert_eq!(config.max_memory_bytes, 64 * 1024 * 1024);
        assert!(config.enable_wasi);
    }

    #[test]
    fn test_map_plugin_result_continue() {
        let result = plugin_abi::PluginResult {
            action: "continue".into(),
            data: None,
        };
        let hook = map_plugin_result_to_hook_result(&result);
        assert!(matches!(hook, HookResult::Continue));
    }

    #[test]
    fn test_map_plugin_result_block() {
        let result = plugin_abi::PluginResult {
            action: "block".into(),
            data: Some("policy violation".into()),
        };
        let hook = map_plugin_result_to_hook_result(&result);
        assert!(matches!(hook, HookResult::Block { reason } if reason == "policy violation"));
    }

    #[test]
    fn test_map_plugin_result_ask() {
        let result = plugin_abi::PluginResult {
            action: "ask".into(),
            data: Some("Are you sure?".into()),
        };
        let hook = map_plugin_result_to_hook_result(&result);
        assert!(matches!(hook, HookResult::Ask { question, .. } if question == "Are you sure?"));
    }

    #[test]
    fn test_map_plugin_result_unknown() {
        let result = plugin_abi::PluginResult {
            action: "unknown".into(),
            data: None,
        };
        let hook = map_plugin_result_to_hook_result(&result);
        assert!(matches!(hook, HookResult::Continue));
    }

    #[tokio::test]
    async fn test_wasm_handler_invalid_handler_type() {
        let handler = WasmHandler::new(PathBuf::from("/tmp"));
        let hook_handler = HookHandler::command("echo");
        let context = HookContext::new(HookEvent::PreToolCall);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await;

        assert!(result.is_err());
    }
}
