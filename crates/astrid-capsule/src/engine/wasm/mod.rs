use async_trait::async_trait;
use extism::{Manifest, PluginBuilder, UserData, Wasm};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::context::CapsuleContext;
use crate::engine::ExecutionEngine;
use crate::engine::wasm::host::register_host_functions;
use crate::engine::wasm::host_state::{HostState, LifecyclePhase};
use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::CapsuleManifest;

pub mod host;
pub mod host_state;
pub(crate) mod tool;

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
    /// Receiver for the readiness signal from the run loop.
    /// Only set for capsules that have a `run()` export.
    /// The Mutex is required because `wait_ready` takes `&self` but we need
    /// to clone the receiver (which marks the current value as seen). We
    /// clone inside the lock and immediately drop it, so concurrent
    /// `wait_ready` calls each get their own independent receiver.
    ready_rx: Option<tokio::sync::Mutex<tokio::sync::watch::Receiver<bool>>>,
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
            ready_rx: None,
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

        let reserved_keys: Vec<String> = wasm_config.keys().cloned().collect();
        let resolved_env =
            super::resolve_env(&self.manifest, ctx, &reserved_keys, "wasm_engine").await?;

        for (key, val) in resolved_env {
            wasm_config.insert(key, serde_json::Value::String(val));
        }

        let (plugin, rx, has_run, ready_rx) = tokio::task::block_in_place(move || {
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

            let secret_store = astrid_storage::build_secret_store(
                &manifest.package.name,
                kv.clone(),
                tokio::runtime::Handle::current(),
            );

            let mut host_state = HostState {
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
                has_uplink_capability: !manifest.uplinks.is_empty(),
                inbound_tx: tx,
                registered_uplinks: Vec::new(),
                lifecycle_phase: None,
                secret_store,
                ready_tx: None,
            };

            // Create the readiness watch channel. The sender goes into
            // HostState so the WASM guest can signal ready via
            // `astrid_signal_ready`. The receiver is stored only when
            // `has_run` is true (see below).
            //
            // Note: for non-run capsules, ready_tx remains in HostState as
            // an orphaned sender (receiver is dropped). This is harmless -
            // watch::Sender::send is a no-op with no receivers, and the
            // allocation is freed when the plugin is dropped. Ideally we'd
            // defer channel creation to after the plugin build (when has_run
            // is known), but Extism's UserData is type-erased and
            // inaccessible after Plugin::build().
            let (ready_tx, ready_rx) = tokio::sync::watch::channel(false);
            host_state.ready_tx = Some(ready_tx);

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

            Ok::<_, CapsuleError>((plugin, rx, has_run, ready_rx))
        })?;

        let plugin_arc = Arc::new(Mutex::new(plugin));

        if has_run {
            self.ready_rx = Some(tokio::sync::Mutex::new(ready_rx));

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
        self.ready_rx = None; // Prevent stale channel observation post-unload
        self.tools.clear();
        Ok(())
    }

    async fn wait_ready(&self, timeout: std::time::Duration) -> crate::capsule::ReadyStatus {
        use crate::capsule::ReadyStatus;

        let Some(rx_mutex) = &self.ready_rx else {
            return ReadyStatus::Ready;
        };
        let mut rx = rx_mutex.lock().await.clone();
        match tokio::time::timeout(timeout, rx.wait_for(|&v| v)).await {
            Ok(Ok(_)) => ReadyStatus::Ready,
            Ok(Err(_)) => ReadyStatus::Crashed, // sender dropped before signaling
            Err(_) => ReadyStatus::Timeout,
        }
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

    fn check_health(&self) -> crate::capsule::CapsuleState {
        if let Some(handle) = &self.run_handle
            && handle.is_finished()
        {
            return crate::capsule::CapsuleState::Failed(
                "WASM run loop exited unexpectedly".into(),
            );
        }
        crate::capsule::CapsuleState::Ready
    }
}

/// Configuration for lifecycle dispatch.
pub struct LifecycleConfig {
    /// The WASM binary bytes.
    pub wasm_bytes: Vec<u8>,
    /// Capsule identifier.
    pub capsule_id: crate::capsule::CapsuleId,
    /// Workspace root directory for VFS.
    pub workspace_root: PathBuf,
    /// Scoped KV store for the capsule.
    pub kv: astrid_storage::ScopedKvStore,
    /// Event bus for IPC (elicit requests flow through this).
    pub event_bus: astrid_events::EventBus,
    /// Plugin configuration values (env vars, etc.).
    pub config: std::collections::HashMap<String, serde_json::Value>,
    /// Secret store for capsule credentials (keychain with KV fallback).
    pub secret_store: std::sync::Arc<dyn astrid_storage::secret::SecretStore>,
}

/// Run a capsule's lifecycle hook (install or upgrade).
///
/// Builds a temporary, short-lived plugin instance with no wall-clock timeout
/// (lifecycle hooks involve human interaction via `elicit`). If the WASM binary
/// does not export the relevant function (`astrid_install` or `astrid_upgrade`),
/// returns `Ok(())` silently.
///
/// # Errors
///
/// Returns an error if the WASM plugin fails to build or the lifecycle hook
/// returns an error.
pub fn run_lifecycle(
    cfg: LifecycleConfig,
    phase: LifecyclePhase,
    previous_version: Option<&str>,
) -> CapsuleResult<()> {
    let export_name = match phase {
        LifecyclePhase::Install => "astrid_install",
        LifecyclePhase::Upgrade => "astrid_upgrade",
    };

    // Build a minimal VFS
    let vfs = astrid_vfs::HostVfs::new();
    let root_handle = astrid_capabilities::DirHandle::new();
    tokio::runtime::Handle::current()
        .block_on(async {
            vfs.register_dir(root_handle.clone(), cfg.workspace_root.clone())
                .await
        })
        .map_err(|e| {
            CapsuleError::UnsupportedEntryPoint(format!(
                "Failed to register VFS directory for lifecycle: {e}"
            ))
        })?;

    let host_state = HostState {
        capsule_uuid: uuid::Uuid::new_v4(),
        caller_context: None,
        capsule_id: cfg.capsule_id.clone(),
        workspace_root: cfg.workspace_root,
        vfs: Arc::new(vfs),
        vfs_root_handle: root_handle,
        global_root: None,
        global_vfs: None,
        global_vfs_root_handle: None,
        upper_dir: None,
        kv: cfg.kv,
        event_bus: cfg.event_bus,
        ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
        subscriptions: std::collections::HashMap::new(),
        next_subscription_id: 1,
        config: cfg.config,
        ipc_publish_patterns: Vec::new(),
        security: None,
        hook_manager: None,
        capsule_registry: None,
        runtime_handle: tokio::runtime::Handle::current(),
        has_uplink_capability: false,
        inbound_tx: None,
        registered_uplinks: Vec::new(),
        cli_socket_listener: None,
        active_streams: std::collections::HashMap::new(),
        next_stream_id: 1,
        lifecycle_phase: Some(phase),
        secret_store: cfg.secret_store,
        ready_tx: None,
    };

    let user_data = UserData::new(host_state);

    let extism_wasm = Wasm::data(cfg.wasm_bytes);
    // No timeout - lifecycle hooks involve human interaction via elicit.
    let extism_manifest = Manifest::new([extism_wasm]).with_memory_max(1024);

    let builder = PluginBuilder::new(extism_manifest).with_wasi(true);
    let builder = register_host_functions(builder, user_data);

    let mut plugin = builder.build().map_err(|e| {
        CapsuleError::UnsupportedEntryPoint(format!(
            "Failed to build Extism plugin for lifecycle: {e}"
        ))
    })?;

    // Check if the export exists - lifecycle hooks are optional
    if !plugin.function_exists(export_name) {
        tracing::debug!(
            capsule = %cfg.capsule_id,
            export = export_name,
            "Capsule does not export lifecycle hook, skipping"
        );
        return Ok(());
    }

    tracing::info!(
        capsule = %cfg.capsule_id,
        phase = ?phase,
        previous_version = previous_version.unwrap_or("(none)"),
        "Running lifecycle hook"
    );

    // Call the lifecycle export
    let input = previous_version.unwrap_or("");
    plugin.call::<&str, ()>(export_name, input).map_err(|e| {
        CapsuleError::ExecutionFailed(format!("lifecycle hook {export_name} failed: {e}"))
    })?;

    tracing::info!(
        capsule = %cfg.capsule_id,
        phase = ?phase,
        "Lifecycle hook completed successfully"
    );

    Ok(())
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
            placeholder: None,
        };
        let field = crate::engine::build_onboarding_field("owner", &def);
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
            placeholder: None,
        };
        let field = crate::engine::build_onboarding_field("apiKey", &def);
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
            placeholder: None,
        };
        let field = crate::engine::build_onboarding_field("network", &def);
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
            placeholder: None,
        };
        let field = crate::engine::build_onboarding_field("someKey", &def);
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
            placeholder: None,
        };
        let field = crate::engine::build_onboarding_field("single", &def);
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
    fn build_onboarding_field_array() {
        let def = crate::manifest::EnvDef {
            env_type: "array".into(),
            request: Some("Enter relay URLs".into()),
            description: Some("Nostr relay endpoints".into()),
            default: None,
            enum_values: vec![],
            placeholder: None,
        };
        let field = crate::engine::build_onboarding_field("relays", &def);
        assert_eq!(
            field.field_type,
            astrid_events::ipc::OnboardingFieldType::Array
        );
        assert_eq!(field.prompt, "Enter relay URLs");
    }

    #[test]
    fn build_onboarding_field_empty_enum_degrades_to_text() {
        let def = crate::manifest::EnvDef {
            env_type: "string".into(),
            request: None,
            description: None,
            default: None,
            enum_values: vec![],
            placeholder: None,
        };
        let field = crate::engine::build_onboarding_field("empty", &def);
        assert_eq!(
            field.field_type,
            astrid_events::ipc::OnboardingFieldType::Text,
            "Empty enum should degrade to text"
        );
    }

    // --- wait_ready / watch channel tests ---

    /// Helper: build a WasmEngine-like wait_ready from a watch receiver.
    async fn wait_ready_from_rx(
        rx: &tokio::sync::Mutex<tokio::sync::watch::Receiver<bool>>,
        timeout: std::time::Duration,
    ) -> crate::capsule::ReadyStatus {
        use crate::capsule::ReadyStatus;
        let mut rx = rx.lock().await.clone();
        match tokio::time::timeout(timeout, rx.wait_for(|&v| v)).await {
            Ok(Ok(_)) => ReadyStatus::Ready,
            Ok(Err(_)) => ReadyStatus::Crashed,
            Err(_) => ReadyStatus::Timeout,
        }
    }

    #[tokio::test]
    async fn wait_ready_returns_ready_when_pre_signaled() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let _ = tx.send(true);
        let rx_mutex = tokio::sync::Mutex::new(rx);
        let status = wait_ready_from_rx(&rx_mutex, std::time::Duration::from_millis(100)).await;
        assert_eq!(status, crate::capsule::ReadyStatus::Ready);
    }

    #[tokio::test]
    async fn wait_ready_returns_timeout_when_never_signaled() {
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let rx_mutex = tokio::sync::Mutex::new(rx);
        let status = wait_ready_from_rx(&rx_mutex, std::time::Duration::from_millis(10)).await;
        assert_eq!(status, crate::capsule::ReadyStatus::Timeout);
    }

    #[tokio::test]
    async fn wait_ready_returns_crashed_when_sender_dropped() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        drop(tx); // simulate capsule crash
        let rx_mutex = tokio::sync::Mutex::new(rx);
        let status = wait_ready_from_rx(&rx_mutex, std::time::Duration::from_millis(100)).await;
        assert_eq!(status, crate::capsule::ReadyStatus::Crashed);
    }

    #[tokio::test]
    async fn wait_ready_returns_ready_when_signaled_after_delay() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let rx_mutex = tokio::sync::Mutex::new(rx);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let _ = tx.send(true);
        });
        let status = wait_ready_from_rx(&rx_mutex, std::time::Duration::from_millis(500)).await;
        assert_eq!(status, crate::capsule::ReadyStatus::Ready);
    }
}
