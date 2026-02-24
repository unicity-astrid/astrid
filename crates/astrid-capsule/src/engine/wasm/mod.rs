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

/// Executes Pure WASM Components and AstridClaw transpiled OpenClaw plugins.
///
/// This engine sandboxes the execution in Extism/Wasmtime and injects the
/// `astrid-sys` Airlocks (host functions) so the component can interact
/// securely with the OS Event Bus and VFS.
pub struct WasmEngine {
    manifest: CapsuleManifest,
    _capsule_dir: PathBuf,
    plugin: Option<Arc<Mutex<extism::Plugin>>>,
}

impl WasmEngine {
    pub fn new(manifest: CapsuleManifest, capsule_dir: PathBuf) -> Self {
        Self {
            manifest,
            _capsule_dir: capsule_dir,
            plugin: None,
        }
    }
}

#[async_trait]
impl ExecutionEngine for WasmEngine {
    async fn load(&mut self, _ctx: &CapsuleContext) -> CapsuleResult<()> {
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

        // We wrap these synchronous operations in block_in_place
        let plugin = tokio::task::block_in_place(|| {
            let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| {
                CapsuleError::UnsupportedEntryPoint(format!("Failed to read WASM: {e}"))
            })?;

            // Build HostState (Minimal for scaffolding)
            let lower_vfs = astrid_vfs::HostVfs::new();
            let upper_vfs = astrid_vfs::HostVfs::new();
            let root_handle = astrid_capabilities::DirHandle::new();
            let overlay_vfs = astrid_vfs::OverlayVfs::new(Box::new(lower_vfs), Box::new(upper_vfs));

            let host_state = HostState {
                capsule_uuid: uuid::Uuid::new_v4(),
                capsule_id: crate::capsule::CapsuleId::from_static(&self.manifest.package.name),
                workspace_root: std::env::current_dir().unwrap_or_default(),
                vfs: Arc::new(overlay_vfs),
                vfs_root_handle: root_handle,
                upper_dir: None,
                kv: astrid_storage::ScopedKvStore::new(
                    Arc::new(astrid_storage::MemoryKvStore::new()),
                    &self.manifest.package.name,
                )
                .unwrap(),
                event_bus: astrid_events::EventBus::with_capacity(128),
                ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
                subscriptions: std::collections::HashMap::new(),
                next_subscription_id: 1,
                config: std::collections::HashMap::new(),
                security: None,
                runtime_handle: tokio::runtime::Handle::current(),
                has_connector_capability: !self.manifest.uplinks.is_empty(),
                inbound_tx: None,
                uplink_buffer: Vec::new(),
                registered_connectors: Vec::new(),
            };

            let user_data = UserData::new(host_state);

            let extism_wasm = Wasm::data(wasm_bytes);
            let extism_manifest = Manifest::new([extism_wasm])
                .with_timeout(std::time::Duration::from_secs(30))
                .with_memory_max(1024); // 64MB

            let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
            let builder = register_host_functions(builder, user_data);

            builder.build().map_err(|e| {
                CapsuleError::UnsupportedEntryPoint(format!("Failed to build Extism plugin: {e}"))
            })
        })?;

        self.plugin = Some(Arc::new(Mutex::new(plugin)));

        Ok(())
    }

    async fn unload(&mut self) -> CapsuleResult<()> {
        info!(
            capsule = %self.manifest.package.name,
            "Unloading WASM component"
        );
        self.plugin = None; // Drop releases WASM memory
        Ok(())
    }
}
