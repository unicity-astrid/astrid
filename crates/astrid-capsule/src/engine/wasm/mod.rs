use async_trait::async_trait;
use extism::{Manifest, PluginBuilder, UserData, Wasm};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::context::CapsuleContext;
use crate::engine::ExecutionEngine;
use crate::engine::wasm::host::register_host_functions;
use crate::engine::wasm::host_state::HostState;
use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::CapsuleManifest;

pub mod host;
pub mod host_state;
pub mod tool;

/// Executes Pure WASM Components and AstridClaw transpiled OpenClaw plugins.
///
/// This engine sandboxes the execution in Extism/Wasmtime and injects the
/// `astrid-sys` Airlocks (host functions) so the component can interact
/// securely with the OS Event Bus and VFS.
pub struct WasmEngine {
    manifest: CapsuleManifest,
    _capsule_dir: PathBuf,
    plugin: Option<Arc<Mutex<extism::Plugin>>>,
    inbound_rx: Option<tokio::sync::mpsc::Receiver<astrid_core::InboundMessage>>,
    tools: Vec<std::sync::Arc<dyn crate::tool::CapsuleTool>>,
}

impl WasmEngine {
    pub fn new(manifest: CapsuleManifest, capsule_dir: PathBuf) -> Self {
        Self {
            manifest,
            _capsule_dir: capsule_dir,
            plugin: None,
            inbound_rx: None,
            tools: Vec::new(),
        }
    }
}

#[async_trait]
impl ExecutionEngine for WasmEngine {
    async fn load(&mut self, ctx: &CapsuleContext) -> CapsuleResult<()> {
        info!(
            capsule = %self.manifest.package.name,
            "Loading Pure WASM component"
        );

        let component = self.manifest.component.as_ref().ok_or_else(|| {
            CapsuleError::UnsupportedEntryPoint(
                "WASM engine requires a component definition".into(),
            )
        })?;

        let wasm_path = if component.entrypoint.is_absolute() {
            component.entrypoint.clone()
        } else {
            self._capsule_dir.join(&component.entrypoint)
        };

        // Clone context components to move into block_in_place
        let workspace_root = ctx.workspace_root.clone();
        let kv = ctx.kv.clone();
        let event_bus = astrid_events::EventBus::clone(&ctx.event_bus);
        let manifest = self.manifest.clone();

        let mut wasm_config = std::collections::HashMap::new();
        for (key, def) in &self.manifest.env {
            if let Ok(Some(val_bytes)) = ctx.kv.get(key).await {
                if let Ok(val) = String::from_utf8(val_bytes) {
                    wasm_config.insert(key.clone(), serde_json::Value::String(val));
                }
            } else if let Some(default_val) = &def.default {
                wasm_config.insert(key.clone(), default_val.clone());
            }
        }

        let (plugin, rx) = tokio::task::block_in_place(move || {
            let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| {
                CapsuleError::UnsupportedEntryPoint(format!("Failed to read WASM: {e}"))
            })?;

            let (tx, rx) = if !manifest.uplinks.is_empty() {
                let (tx, rx) = tokio::sync::mpsc::channel(128);
                (Some(tx), Some(rx))
            } else {
                (None, None)
            };

            // Build HostState
            let lower_vfs = astrid_vfs::HostVfs::new();
            let upper_vfs = astrid_vfs::HostVfs::new();
            let root_handle = astrid_capabilities::DirHandle::new();

            tokio::runtime::Handle::current()
                .block_on(async {
                    lower_vfs
                        .register_dir(root_handle.clone(), workspace_root.clone())
                        .await?;
                    upper_vfs
                        .register_dir(root_handle.clone(), workspace_root.clone())
                        .await?;
                    Ok::<(), astrid_vfs::VfsError>(())
                })
                .map_err(|e| {
                    CapsuleError::UnsupportedEntryPoint(format!(
                        "Failed to register VFS directory: {e}"
                    ))
                })?;

            // NOTE: In Phase 4, OverlayVfs upper and lower layers share the same physical
            // workspace root, meaning CoW semantics act as a direct pass-through.
            // In Phase 5+, upper_vfs will point to a temporary session overlay directory.
            let overlay_vfs = astrid_vfs::OverlayVfs::new(Box::new(lower_vfs), Box::new(upper_vfs));

            let next_subscription_id = 1;
            let security_gate =
                Arc::new(crate::security::ManifestSecurityGate::new(manifest.clone()));

            let host_state = HostState {
                capsule_uuid: uuid::Uuid::new_v4(),
                capsule_id: crate::capsule::CapsuleId::new(&manifest.package.name)
                    .map_err(|e| CapsuleError::UnsupportedEntryPoint(e.to_string()))?,
                workspace_root,
                vfs: Arc::new(overlay_vfs),
                vfs_root_handle: root_handle,
                upper_dir: None,
                kv,
                event_bus,
                ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
                subscriptions: std::collections::HashMap::new(),
                next_subscription_id,
                config: wasm_config,
                security: Some(security_gate),
                runtime_handle: tokio::runtime::Handle::current(),
                has_connector_capability: !manifest.uplinks.is_empty(),
                inbound_tx: tx,
                registered_connectors: Vec::new(),
            };

            let user_data = UserData::new(host_state);

            let extism_wasm = Wasm::data(wasm_bytes);
            let extism_manifest = Manifest::new([extism_wasm])
                .with_timeout(std::time::Duration::from_secs(10)) // Reduced from 30s
                .with_memory_max(1024); // 64MB

            // We will set instruction limits (fuel) when Wasmtime natively exposes it through Extism,
            // but for now, the 10-second wall-clock timeout acts as our gas limit.

            let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
            let builder = register_host_functions(builder, user_data);

            let plugin = builder.build().map_err(|e| {
                CapsuleError::UnsupportedEntryPoint(format!("Failed to build Extism plugin: {e}"))
            })?;

            Ok::<_, CapsuleError>((plugin, rx))
        })?;

        let plugin_arc = Arc::new(Mutex::new(plugin));

        let mut tools: Vec<std::sync::Arc<dyn crate::tool::CapsuleTool>> = Vec::new();
        for t in &self.manifest.tools {
            tools.push(Arc::new(tool::WasmCapsuleTool::new(
                t.name.clone(),
                t.description.clone(),
                t.input_schema.clone(),
                Arc::clone(&plugin_arc),
            )));
        }

        self.plugin = Some(plugin_arc);
        self.inbound_rx = rx;
        self.tools = tools;

        Ok(())
    }

    async fn unload(&mut self) -> CapsuleResult<()> {
        info!(
            capsule = %self.manifest.package.name,
            "Unloading WASM component"
        );
        self.plugin = None; // Drop releases WASM memory
        self.tools.clear();
        Ok(())
    }

    fn take_inbound_rx(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<astrid_core::InboundMessage>> {
        self.inbound_rx.take()
    }

    fn tools(&self) -> &[std::sync::Arc<dyn crate::tool::CapsuleTool>] {
        &self.tools
    }
}
