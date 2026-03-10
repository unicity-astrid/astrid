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
    run_handle: Option<tokio::task::JoinHandle<()>>,
}

impl WasmEngine {
    pub fn new(manifest: CapsuleManifest, capsule_dir: PathBuf) -> Self {
        Self {
            manifest,
            _capsule_dir: capsule_dir,
            plugin: None,
            inbound_rx: None,
            tools: Vec::new(),
            run_handle: None,
        }
    }

    /// Build an `OnboardingField` from a manifest `EnvDef`.
    fn build_onboarding_field(
        key: &str,
        def: &crate::manifest::EnvDef,
    ) -> astrid_events::ipc::OnboardingField {
        use astrid_events::ipc::OnboardingFieldType;

        let field_type = if def.env_type == "secret" {
            if !def.enum_values.is_empty() {
                tracing::warn!(
                    key = %key,
                    "Secret field has enum_values — ignoring enum and using masked input"
                );
            }
            OnboardingFieldType::Secret
        } else if def.enum_values.len() > 1 {
            OnboardingFieldType::Enum(def.enum_values.clone())
        } else {
            // Empty or single-choice enums degrade to text input.
            OnboardingFieldType::Text
        };

        let mut default = def.default.as_ref().and_then(|d| match d {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Null => None,
            other => Some(other.to_string()),
        });

        // Single-choice enums degrade to text — auto-fill the sole valid value.
        if def.enum_values.len() == 1 && default.is_none() {
            default = Some(def.enum_values[0].clone());
        }

        let prompt = def
            .request
            .clone()
            .unwrap_or_else(|| format!("Please enter value for {key}"));

        astrid_events::ipc::OnboardingField {
            key: key.to_string(),
            prompt,
            description: def.description.clone(),
            field_type,
            default,
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

        let mut onboarding_fields = Vec::new();

        // Collect reserved kernel-injected keys so the env loop cannot override them.
        let reserved_keys: Vec<String> = wasm_config.keys().cloned().collect();

        for (key, def) in &self.manifest.env {
            // Reject manifest [env] entries that collide with kernel-injected config.
            if reserved_keys.iter().any(|k| k == key) {
                tracing::warn!(
                    capsule = %self.manifest.package.name,
                    key = %key,
                    "Capsule manifest [env] declares reserved key — ignoring"
                );
                continue;
            }
            if let Ok(Some(val_bytes)) = ctx.kv.get(key).await {
                if let Ok(val) = String::from_utf8(val_bytes) {
                    wasm_config.insert(key.clone(), serde_json::Value::String(val));
                } else {
                    onboarding_fields.push(Self::build_onboarding_field(key, def));
                }
            } else if let Some(default_val) = &def.default {
                // Manifest declares a default — inject silently without prompting.
                wasm_config.insert(key.clone(), default_val.clone());
            } else {
                onboarding_fields.push(Self::build_onboarding_field(key, def));
            }
        }

        if !onboarding_fields.is_empty() {
            let missing_names: Vec<String> =
                onboarding_fields.iter().map(|f| f.key.clone()).collect();
            let msg = astrid_events::ipc::IpcMessage::new(
                "system.onboarding.required",
                astrid_events::ipc::IpcPayload::OnboardingRequired {
                    capsule_id: self.manifest.package.name.clone(),
                    fields: onboarding_fields,
                },
                uuid::Uuid::nil(), // Broadcast or global event for onboarding
            );
            let _ = ctx.event_bus.publish(astrid_events::AstridEvent::Ipc {
                metadata: astrid_events::EventMetadata::new("wasm_engine"),
                message: msg,
            });

            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "Missing required environment variables: {}",
                missing_names.join(", ")
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
            let global_root = ctx.global_root.clone();

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

            // Set up the global VFS (backed by ~/.astrid/shared/). Writes go
            // directly to disk — there is no OverlayVfs CoW layer here,
            // unlike the workspace VFS. Only mount if the directory exists
            // to avoid failing capsule load on fresh installs.
            let (global_vfs, global_vfs_root_handle): (
                Option<Arc<dyn astrid_vfs::Vfs>>,
                Option<astrid_capabilities::DirHandle>,
            ) = if let Some(ref g_root) = global_root {
                if g_root.exists() {
                    let g_vfs = astrid_vfs::HostVfs::new();
                    let g_handle = astrid_capabilities::DirHandle::new();
                    tokio::runtime::Handle::current()
                        .block_on(async {
                            g_vfs.register_dir(g_handle.clone(), g_root.clone()).await
                        })
                        .map_err(|e| {
                            CapsuleError::UnsupportedEntryPoint(format!(
                                "Failed to register global VFS directory: {e}"
                            ))
                        })?;
                    (
                        Some(Arc::new(g_vfs) as Arc<dyn astrid_vfs::Vfs>),
                        Some(g_handle),
                    )
                } else {
                    tracing::warn!(
                        global_root = %g_root.display(),
                        "global:// VFS not mounted: directory does not exist. \
                         Capsules requesting global:// paths will receive errors \
                         until the directory is created and the kernel is restarted."
                    );
                    (None, None)
                }
            } else {
                (None, None)
            };

            // TODO: OverlayVfs upper and lower layers currently share the same physical
            // workspace root, meaning CoW semantics act as a direct pass-through.
            // upper_vfs should point to a temporary session overlay directory.
            let overlay_vfs = astrid_vfs::OverlayVfs::new(Box::new(lower_vfs), Box::new(upper_vfs));

            let next_subscription_id = 1;
            // Only resolve global:// in the gate if we actually mounted the VFS.
            // Otherwise the gate would approve paths the VFS can't serve.
            let gate_global_root = if global_vfs.is_some() {
                global_root.clone()
            } else {
                None
            };
            let security_gate = Arc::new(crate::security::ManifestSecurityGate::new(
                manifest.clone(),
                workspace_root.clone(),
                gate_global_root,
            ));

            let host_state = HostState {
                capsule_uuid: uuid::Uuid::new_v4(),
                caller_context: None,
                capsule_id: crate::capsule::CapsuleId::new(&manifest.package.name)
                    .map_err(|e| CapsuleError::UnsupportedEntryPoint(e.to_string()))?,
                workspace_root,
                vfs: Arc::new(overlay_vfs),
                vfs_root_handle: root_handle,
                global_root,
                global_vfs,
                global_vfs_root_handle,
                upper_dir: None,
                kv,
                event_bus,
                ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
                subscriptions: std::collections::HashMap::new(),
                next_subscription_id,
                config: wasm_config,
                ipc_publish_patterns: manifest.capabilities.ipc_publish.clone(),
                // Only provide the CLI socket listener if the capsule declares net_bind.
                // This prevents unauthorized capsules from even seeing the listener.
                cli_socket_listener: if manifest.capabilities.net_bind.is_empty() {
                    None
                } else {
                    ctx.cli_socket_listener.clone()
                },
                active_streams: std::collections::HashMap::new(),
                next_stream_id: 1,
                security: Some(security_gate),
                hook_manager: None, // Will be injected by Gateway
                capsule_registry: ctx.capsule_registry.clone(),
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
            // Must spawn on a worker thread (not spawn_blocking) because WASM
            // host functions (fs, http, kv, etc.) use block_in_place internally,
            // which panics on spawn_blocking threads. Requires multi-thread runtime.
            self.run_handle = Some(tokio::task::spawn(async move {
                tracing::info!(capsule = %capsule_name, "Starting background WASM run loop");
                tokio::task::block_in_place(|| {
                    let mut p = match plugin_arc.lock() {
                        Ok(guard) => guard,
                        Err(e) => {
                            tracing::error!(capsule = %capsule_name, error = %e, "WASM plugin lock was poisoned");
                            return;
                        },
                    };
                    if let Err(e) = p.call::<(), ()>("run", ()) {
                        tracing::error!(capsule = %capsule_name, error = %e, "WASM background loop failed");
                    }
                });
            }));
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
        if let Some(handle) = self.run_handle.take() {
            handle.abort();
        }
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

        // block_in_place is required because Extism host functions (fs, http,
        // kv, etc.) also call block_in_place internally during plugin.call().
        // The caller MUST invoke this from a Tokio worker thread (e.g. via
        // tokio::task::spawn), never from spawn_blocking.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Poisons a mutex by panicking while holding the lock.
    fn poison_mutex<T: Send + 'static>(mutex: &Arc<Mutex<T>>) {
        let m = Arc::clone(mutex);
        let _ = std::thread::spawn(move || {
            let _guard = m.lock().unwrap();
            panic!("intentional panic to poison mutex");
        })
        .join();
    }

    /// Verifies that a poisoned mutex in the run-loop pattern completes
    /// without panicking — matching the lock error handling in `load()`.
    #[tokio::test]
    async fn poisoned_lock_in_run_loop_does_not_panic() {
        let plugin_arc: Arc<Mutex<String>> = Arc::new(Mutex::new("fake_plugin".into()));
        poison_mutex(&plugin_arc);

        let handle = tokio::task::spawn_blocking(move || {
            let capsule_name = "test-capsule";
            let _p = match plugin_arc.lock() {
                Ok(guard) => guard,
                Err(e) => {
                    tracing::error!(capsule = %capsule_name, error = %e, "WASM plugin lock was poisoned");
                    return false;
                },
            };
            true
        });

        let result = handle.await;
        assert!(result.is_ok(), "spawn_blocking should not panic");
        assert!(!result.unwrap(), "should have taken the poison error path");
    }

    /// Verifies that a poisoned mutex in the invoke_interceptor pattern
    /// returns a WasmError instead of panicking — matching lines 320-322.
    #[test]
    fn poisoned_lock_in_interceptor_returns_error() {
        let plugin: Arc<Mutex<String>> = Arc::new(Mutex::new("fake_plugin".into()));
        poison_mutex(&plugin);

        let result: CapsuleResult<Vec<u8>> = plugin
            .lock()
            .map_err(|e| CapsuleError::WasmError(format!("plugin lock poisoned: {e}")))
            .map(|_guard| vec![]);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, CapsuleError::WasmError(_)),
            "expected WasmError, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("poisoned"),
            "error message should mention poisoning: {msg}"
        );
    }

    #[test]
    fn build_onboarding_field_text() {
        let def = crate::manifest::EnvDef {
            env_type: "string".into(),
            request: Some("Enter owner address".into()),
            description: Some("The wallet address".into()),
            default: None,
            enum_values: vec![],
        };
        let field = WasmEngine::build_onboarding_field("owner", &def);
        assert_eq!(field.key, "owner");
        assert_eq!(field.prompt, "Enter owner address");
        assert_eq!(field.description.as_deref(), Some("The wallet address"));
        assert_eq!(
            field.field_type,
            astrid_events::ipc::OnboardingFieldType::Text
        );
        assert!(field.default.is_none());
    }

    #[test]
    fn build_onboarding_field_secret() {
        let def = crate::manifest::EnvDef {
            env_type: "secret".into(),
            request: None,
            description: None,
            default: None,
            enum_values: vec!["a".into()], // enum_values ignored for secrets
        };
        let field = WasmEngine::build_onboarding_field("apiKey", &def);
        assert_eq!(
            field.field_type,
            astrid_events::ipc::OnboardingFieldType::Secret
        );
    }

    #[test]
    fn build_onboarding_field_enum_with_default() {
        let def = crate::manifest::EnvDef {
            env_type: "string".into(),
            request: Some("Select network".into()),
            description: None,
            default: Some(serde_json::json!("testnet")),
            enum_values: vec!["testnet".into(), "mainnet".into()],
        };
        let field = WasmEngine::build_onboarding_field("network", &def);
        assert_eq!(
            field.field_type,
            astrid_events::ipc::OnboardingFieldType::Enum(vec!["testnet".into(), "mainnet".into()])
        );
        assert_eq!(field.default.as_deref(), Some("testnet"));
    }

    #[test]
    fn build_onboarding_field_fallback_prompt() {
        let def = crate::manifest::EnvDef {
            env_type: "string".into(),
            request: None,
            description: None,
            default: None,
            enum_values: vec![],
        };
        let field = WasmEngine::build_onboarding_field("someKey", &def);
        assert_eq!(field.prompt, "Please enter value for someKey");
    }

    #[test]
    fn build_onboarding_field_single_enum_degrades_to_text_with_autofill() {
        let def = crate::manifest::EnvDef {
            env_type: "string".into(),
            request: None,
            description: None,
            default: None,
            enum_values: vec!["only".into()],
        };
        let field = WasmEngine::build_onboarding_field("single", &def);
        assert_eq!(
            field.field_type,
            astrid_events::ipc::OnboardingFieldType::Text,
            "Single-choice enum should degrade to text"
        );
        assert_eq!(
            field.default.as_deref(),
            Some("only"),
            "Single-choice enum should auto-fill the sole valid value"
        );
    }

    #[test]
    fn build_onboarding_field_empty_enum_degrades_to_text() {
        let def = crate::manifest::EnvDef {
            env_type: "string".into(),
            request: None,
            description: None,
            default: None,
            enum_values: vec![],
        };
        let field = WasmEngine::build_onboarding_field("empty", &def);
        assert_eq!(
            field.field_type,
            astrid_events::ipc::OnboardingFieldType::Text,
            "Empty enum should degrade to text"
        );
    }
}
