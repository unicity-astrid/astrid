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
    tools: Vec<Arc<dyn crate::tool::CapsuleTool>>,
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

        let component = self.manifest.components.first().ok_or_else(|| {
            CapsuleError::UnsupportedEntryPoint(
                "WASM engine requires at least one component definition".into(),
            )
        })?;

        let wasm_path = if component.path.is_absolute() {
            component.path.clone()
        } else {
            self._capsule_dir.join(&component.path)
        };

        // Clone context components to move into block_in_place
        let workspace_root = ctx.workspace_root.clone();
        let kv = ctx.kv.clone();
        let event_bus = astrid_events::EventBus::clone(&ctx.event_bus);
        let manifest = self.manifest.clone();

        let mut wasm_config = std::collections::HashMap::new();

        // Inject the kernel socket path so capsules can discover it via
        // `sys::socket_path()` instead of hardcoding.
        if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            wasm_config.insert(
                "ASTRID_SOCKET_PATH".to_string(),
                serde_json::Value::String(home.socket_path().to_string_lossy().into_owned()),
            );
        }

        let mut missing_keys = Vec::new();
        let mut prompts = std::collections::HashMap::new();

        for (key, def) in &self.manifest.env {
            if let Ok(Some(val_bytes)) = ctx.kv.get(key).await {
                if let Ok(val) = String::from_utf8(val_bytes) {
                    wasm_config.insert(key.clone(), serde_json::Value::String(val));
                } else {
                    missing_keys.push(key.clone());
                    if let Some(req) = &def.request {
                        prompts.insert(key.clone(), req.clone());
                    }
                }
            } else if let Some(default_val) = &def.default {
                wasm_config.insert(key.clone(), default_val.clone());
            } else {
                // Key is missing and has no default
                missing_keys.push(key.clone());
                if let Some(req) = &def.request {
                    prompts.insert(key.clone(), req.clone());
                }
            }
        }

        if !missing_keys.is_empty() {
            let msg = astrid_events::ipc::IpcMessage::new(
                "system.onboarding.required",
                astrid_events::ipc::IpcPayload::OnboardingRequired {
                    capsule_id: self.manifest.package.name.clone(),
                    missing_keys: missing_keys.clone(),
                    prompts,
                },
                uuid::Uuid::nil(), // Broadcast or global event for onboarding
            );
            let _ = ctx.event_bus.publish(astrid_events::AstridEvent::Ipc {
                metadata: astrid_events::EventMetadata::new("wasm_engine"),
                message: msg,
            });

            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "Missing required environment variables: {}",
                missing_keys.join(", ")
            )));
        }

        let (plugin, rx, has_run) = tokio::task::block_in_place(move || {
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
                caller_context: None,
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
                cli_socket_listener: ctx.cli_socket_listener.clone(),
                active_streams: std::collections::HashMap::new(),
                next_stream_id: 1,
                security: Some(security_gate),
                hook_manager: None, // Will be injected by Gateway
                runtime_handle: tokio::runtime::Handle::current(),
                has_connector_capability: !manifest.uplinks.is_empty(),
                inbound_tx: tx,
                registered_connectors: Vec::new(),
            };

            let user_data = UserData::new(host_state);

            let extism_wasm = Wasm::data(wasm_bytes);
            let mut extism_manifest = Manifest::new([extism_wasm]).with_memory_max(1024); // 64MB

            // Long-lived capsules (uplinks, cron, daemons) must not have a wall-clock
            // timeout. Short-lived tool capsules get a 10-second safety timeout.
            let is_daemon = !manifest.uplinks.is_empty()
                || !manifest.cron_jobs.is_empty()
                || manifest.capabilities.uplink;
            if !is_daemon {
                extism_manifest = extism_manifest.with_timeout(std::time::Duration::from_secs(10));
            }

            let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
            let builder = register_host_functions(builder, user_data);

            let plugin = builder.build().map_err(|e| {
                CapsuleError::UnsupportedEntryPoint(format!("Failed to build Extism plugin: {e}"))
            })?;

            let has_run = plugin.function_exists("run");

            Ok::<_, CapsuleError>((plugin, rx, has_run))
        })?;

        let plugin_arc = Arc::new(Mutex::new(plugin));

        if has_run {
            // The run loop holds the plugin mutex for its entire lifetime.
            // We must NOT store the plugin in self.plugin, because the
            // dispatcher's invoke_interceptor() would try to acquire the same
            // mutex — causing a deadlock. Run-loop capsules handle events
            // internally via ipc::subscribe, so they don't need host-side
            // interceptor dispatch.
            if !self.manifest.interceptors.is_empty() {
                tracing::warn!(
                    capsule = %self.manifest.package.name,
                    "Capsule declares both run() and [[interceptor]] entries. \
                     Interceptors will NOT be dispatched for run-loop capsules \
                     (plugin is exclusively held by the run loop). Move event \
                     handling into the run() function via ipc::subscribe instead."
                );
            }
            let capsule_name = self.manifest.package.name.clone();
            tokio::task::spawn_blocking(move || {
                tracing::info!(capsule = %capsule_name, "Starting background WASM run loop");
                let mut p = plugin_arc.lock().expect("WASM plugin lock was poisoned");
                if let Err(e) = p.call::<(), ()>("run", ()) {
                    tracing::error!(capsule = %capsule_name, error = %e, "WASM background loop failed");
                }
            });
            // plugin_arc moved into the spawn — self.plugin stays None.
        } else {
            let mut tools: Vec<Arc<dyn crate::tool::CapsuleTool>> = Vec::new();
            for t in &self.manifest.tools {
                tools.push(Arc::new(tool::WasmCapsuleTool::new(
                    t.name.clone(),
                    t.description.clone(),
                    t.input_schema.clone(),
                    Arc::clone(&plugin_arc),
                )));
            }
            self.tools = tools;
            self.plugin = Some(plugin_arc);
        }
        self.inbound_rx = rx;

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

    fn tools(&self) -> &[Arc<dyn crate::tool::CapsuleTool>] {
        &self.tools
    }

    fn invoke_interceptor(&self, action: &str, payload: &[u8]) -> CapsuleResult<Vec<u8>> {
        let plugin = self
            .plugin
            .as_ref()
            .ok_or_else(|| CapsuleError::ExecutionFailed("plugin not loaded".into()))?;

        // Build the same __AstridToolRequest the macro expects:
        // { "name": "<action>", "arguments": [<payload bytes>] }
        let request = serde_json::json!({
            "name": action,
            "arguments": payload,
        });
        let input = serde_json::to_vec(&request).map_err(|e| {
            CapsuleError::ExecutionFailed(format!("failed to serialize interceptor request: {e}"))
        })?;

        tokio::task::block_in_place(|| {
            let mut plugin = plugin
                .lock()
                .map_err(|e| CapsuleError::WasmError(format!("plugin lock poisoned: {e}")))?;
            plugin
                .call::<&[u8], Vec<u8>>("astrid_hook_trigger", &input)
                .map_err(|e| CapsuleError::WasmError(format!("astrid_hook_trigger failed: {e:?}")))
        })
    }
}
