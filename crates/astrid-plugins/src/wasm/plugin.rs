//! WASM plugin implementation backed by Extism.
//!
//! [`WasmPlugin`] implements the [`Plugin`](crate::Plugin) trait, managing the
//! lifecycle of an Extism WASM module. It loads `.wasm` files, verifies their
//! blake3 hash (if provided), registers host functions, and discovers tools
//! via the `describe-tools` guest export.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use extism::{Manifest, PluginBuilder, UserData, Wasm};

use astrid_core::plugin_abi::ToolDefinition;

use crate::context::PluginContext;
use crate::error::{PluginError, PluginResult};
use crate::manifest::{PluginEntryPoint, PluginManifest};
use crate::plugin::{Plugin, PluginId, PluginState};
use crate::security::PluginSecurityGate;
use crate::tool::PluginTool;
use crate::wasm::host_functions::register_host_functions;
use crate::wasm::host_state::HostState;
use crate::wasm::tool::WasmPluginTool;

/// Configuration from [`WasmPluginLoader`](super::loader::WasmPluginLoader).
///
/// Debug is implemented manually because `dyn PluginSecurityGate` is not `Debug`.
#[derive(Clone)]
pub struct WasmPluginConfig {
    /// Optional security gate for host function authorization.
    pub security: Option<Arc<dyn PluginSecurityGate>>,
    /// Maximum WASM linear memory in bytes.
    pub max_memory_bytes: u64,
    /// Maximum execution time per call.
    pub max_execution_time: Duration,
    /// If true, reject WASM modules that don't specify a hash in their manifest.
    pub require_hash: bool,
}

impl std::fmt::Debug for WasmPluginConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmPluginConfig")
            .field("has_security", &self.security.is_some())
            .field("max_memory_bytes", &self.max_memory_bytes)
            .field("max_execution_time", &self.max_execution_time)
            .field("require_hash", &self.require_hash)
            .finish()
    }
}

/// A plugin backed by an Extism WASM module.
pub struct WasmPlugin {
    id: PluginId,
    manifest: PluginManifest,
    state: PluginState,
    config: WasmPluginConfig,
    /// The Extism plugin instance (created during load).
    extism_plugin: Option<Arc<Mutex<extism::Plugin>>>,
    /// Tools discovered from the guest's `describe-tools` export.
    tools: Vec<Arc<dyn PluginTool>>,
}

impl WasmPlugin {
    /// Create a new `WasmPlugin` in the `Unloaded` state.
    pub(crate) fn new(manifest: PluginManifest, config: WasmPluginConfig) -> Self {
        let id = manifest.id.clone();
        Self {
            id,
            manifest,
            state: PluginState::Unloaded,
            config,
            extism_plugin: None,
            tools: Vec::new(),
        }
    }
}

#[async_trait]
impl Plugin for WasmPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn state(&self) -> PluginState {
        self.state.clone()
    }

    async fn load(&mut self, ctx: &PluginContext) -> PluginResult<()> {
        self.state = PluginState::Loading;

        match self.do_load(ctx) {
            Ok(()) => {
                self.state = PluginState::Ready;
                Ok(())
            },
            Err(e) => {
                let msg = e.to_string();
                self.state = PluginState::Failed(msg);
                Err(e)
            },
        }
    }

    async fn unload(&mut self) -> PluginResult<()> {
        self.state = PluginState::Unloading;
        self.tools.clear();
        self.extism_plugin = None;
        self.state = PluginState::Unloaded;
        Ok(())
    }

    fn tools(&self) -> &[Arc<dyn PluginTool>] {
        &self.tools
    }
}

impl WasmPlugin {
    /// Internal load logic. Separated so we can catch errors and set `Failed` state.
    fn do_load(&mut self, ctx: &PluginContext) -> PluginResult<()> {
        // 1. Resolve WASM file path
        let (wasm_path, expected_hash) = match &self.manifest.entry_point {
            PluginEntryPoint::Wasm { path, hash } => (path.clone(), hash.clone()),
            other @ PluginEntryPoint::Mcp { .. } => {
                return Err(PluginError::LoadFailed {
                    plugin_id: self.id.clone(),
                    message: format!("expected Wasm entry point, got: {other:?}"),
                });
            },
        };

        // If path is relative, resolve relative to workspace root
        let resolved_path = if wasm_path.is_absolute() {
            wasm_path
        } else {
            ctx.workspace_root.join(&wasm_path)
        };

        // 2. Read WASM bytes
        let wasm_bytes = std::fs::read(&resolved_path).map_err(|e| PluginError::LoadFailed {
            plugin_id: self.id.clone(),
            message: format!("failed to read WASM file {}: {e}", resolved_path.display()),
        })?;

        // 3. Hash verification
        verify_hash(
            &wasm_bytes,
            expected_hash.as_deref(),
            &self.id,
            self.config.require_hash,
        )?;

        // 4. Build HostState
        let host_state = HostState {
            plugin_id: self.id.clone(),
            workspace_root: ctx.workspace_root.clone(),
            kv: ctx.kv.clone(),
            config: ctx.config.clone(),
            security: self.config.security.clone(),
            runtime_handle: tokio::runtime::Handle::current(),
        };
        let user_data = UserData::new(host_state);

        // 5. Build Extism Manifest
        let extism_wasm = Wasm::data(wasm_bytes);
        let mut extism_manifest = Manifest::new([extism_wasm]);
        extism_manifest = extism_manifest.with_timeout(self.config.max_execution_time);
        // WASM pages are 64KB each; cap at u32::MAX pages if the byte limit is very large
        let pages = self.config.max_memory_bytes / (64 * 1024);
        let max_pages = u32::try_from(pages).unwrap_or(u32::MAX);
        extism_manifest = extism_manifest.with_memory_max(max_pages);

        // 6. Build Extism Plugin
        let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
        let builder = register_host_functions(builder, user_data);
        let mut plugin = builder
            .build()
            .map_err(|e| PluginError::WasmError(format!("failed to build Extism plugin: {e}")))?;

        // 7. Discover tools via `describe-tools` export
        let tools = discover_tools(&mut plugin)?;
        let plugin_arc = Arc::new(Mutex::new(plugin));

        let wasm_tools: Vec<Arc<dyn PluginTool>> = tools
            .into_iter()
            .map(|td| {
                let schema: serde_json::Value =
                    serde_json::from_str(&td.input_schema).unwrap_or(serde_json::json!({}));
                Arc::new(WasmPluginTool::new(
                    td.name,
                    td.description,
                    schema,
                    Arc::clone(&plugin_arc),
                )) as Arc<dyn PluginTool>
            })
            .collect();

        self.extism_plugin = Some(plugin_arc);
        self.tools = wasm_tools;

        Ok(())
    }
}

/// Verify WASM module hash if an expected hash is provided.
///
/// If `require_hash` is true and no hash is specified in the manifest,
/// loading is rejected. This enforces hash verification in production.
fn verify_hash(
    wasm_bytes: &[u8],
    expected: Option<&str>,
    plugin_id: &PluginId,
    require_hash: bool,
) -> PluginResult<()> {
    match expected {
        Some(expected_hex) => {
            let actual_hex = blake3::hash(wasm_bytes).to_hex().to_string();
            if actual_hex != expected_hex {
                return Err(PluginError::HashMismatch {
                    expected: expected_hex.to_string(),
                    actual: actual_hex,
                });
            }
            tracing::debug!(plugin = %plugin_id, "WASM module hash verified");
        },
        None if require_hash => {
            return Err(PluginError::LoadFailed {
                plugin_id: plugin_id.clone(),
                message: "WASM module hash required but not specified in manifest".into(),
            });
        },
        None => {
            tracing::warn!(
                plugin = %plugin_id,
                "WASM module hash not specified — module integrity not verified"
            );
        },
    }
    Ok(())
}

/// Call the guest's `describe-tools` export and parse the result.
fn discover_tools(plugin: &mut extism::Plugin) -> PluginResult<Vec<ToolDefinition>> {
    // describe-tools takes no input (empty string) and returns JSON array
    let result = plugin
        .call::<&str, String>("describe-tools", "")
        .map_err(|e| PluginError::WasmError(format!("describe-tools call failed: {e}")))?;

    let definitions: Vec<ToolDefinition> = serde_json::from_str(&result).map_err(|e| {
        PluginError::WasmError(format!("failed to parse describe-tools output: {e}"))
    })?;

    Ok(definitions)
}

impl std::fmt::Debug for WasmPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmPlugin")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("tool_count", &self.tools.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn hash_verification_match() {
        let data = b"hello world";
        let expected = blake3::hash(data).to_hex().to_string();
        let id = PluginId::from_static("test");
        assert!(verify_hash(data, Some(&expected), &id, false).is_ok());
    }

    #[test]
    fn hash_verification_mismatch() {
        let data = b"hello world";
        let id = PluginId::from_static("test");
        let result = verify_hash(
            data,
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            &id,
            false,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            PluginError::HashMismatch { expected, actual } => {
                assert_eq!(
                    expected,
                    "0000000000000000000000000000000000000000000000000000000000000000"
                );
                assert!(!actual.is_empty());
            },
            other => panic!("expected HashMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn hash_verification_none_is_ok() {
        let data = b"hello world";
        let id = PluginId::from_static("test");
        assert!(verify_hash(data, None, &id, false).is_ok());
    }

    #[test]
    fn hash_verification_none_rejected_when_required() {
        let data = b"hello world";
        let id = PluginId::from_static("test");
        let result = verify_hash(data, None, &id, true);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PluginError::LoadFailed { .. }
        ));
    }

    #[test]
    fn wasm_plugin_starts_unloaded() {
        let manifest = PluginManifest {
            id: PluginId::from_static("test"),
            name: "Test".into(),
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
        };
        let config = WasmPluginConfig {
            security: None,
            max_memory_bytes: 64 * 1024 * 1024,
            max_execution_time: Duration::from_secs(30),
            require_hash: false,
        };
        let plugin = WasmPlugin::new(manifest, config);
        assert_eq!(plugin.state(), PluginState::Unloaded);
        assert!(plugin.tools().is_empty());
    }
}
