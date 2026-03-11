#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(clippy::module_name_repetitions)]

//! Astrid Kernel - The core execution engine and IPC router.
//!
//! The Kernel is a pure, decentralized WASM runner. It contains no business
//! logic, no cognitive loops, and no network servers. Its sole responsibility
//! is to instantiate `astrid_events::EventBus`, load `.capsule` files into
//! the Extism sandbox, and route IPC bytes between them.

/// The Management API router listening to the `EventBus`.
pub mod kernel_router;
/// The Unix Domain Socket manager.
pub mod socket;

use astrid_capabilities::DirHandle;
use astrid_capsule::registry::CapsuleRegistry;
use astrid_core::SessionId;
use astrid_events::EventBus;
use astrid_mcp::{McpClient, ServerManager, ServersConfig};
use astrid_vfs::{HostVfs, OverlayVfs, Vfs};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// The core Operating System Kernel.
pub struct Kernel {
    /// The unique identifier for this kernel session.
    pub session_id: SessionId,
    /// The global IPC message bus.
    pub event_bus: Arc<EventBus>,
    /// The process manager (loaded WASM capsules).
    pub capsules: Arc<RwLock<CapsuleRegistry>>,
    /// The MCP native process manager.
    pub mcp_client: McpClient,
    /// The global Virtual File System mount.
    pub vfs: Arc<dyn Vfs>,
    /// The global physical root handle (cap-std) for the VFS.
    pub vfs_root_handle: DirHandle,
    /// The physical path the VFS is mounted to.
    pub workspace_root: PathBuf,
    /// The global shared resources directory (`~/.astrid/shared/`). Capsules
    /// declaring `fs_read = ["global://"]` can read files under this root.
    /// Scoped to `shared/` so that keys, databases, and capsule .env files in
    /// `~/.astrid/` are NOT accessible. Write access is intentionally not
    /// granted to any shipped capsule.
    pub global_root: Option<PathBuf>,
    /// The natively bound Unix Socket for the CLI proxy.
    pub cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    /// Shared KV store backing all capsule-scoped stores and kernel state.
    pub kv: Arc<astrid_storage::MemoryKvStore>,
}

impl Kernel {
    /// Boot a new Kernel instance mounted at the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the VFS mount paths cannot be registered.
    pub async fn new(
        session_id: SessionId,
        workspace_root: PathBuf,
    ) -> Result<Arc<Self>, std::io::Error> {
        use astrid_core::dirs::AstridHome;

        let event_bus = Arc::new(EventBus::new());
        let capsules = Arc::new(RwLock::new(CapsuleRegistry::new()));

        // Resolve the global shared directory for the `global://` VFS scheme.
        // Scoped to `~/.astrid/shared/` — NOT the full `~/.astrid/` root — so
        // capsules cannot access keys, databases, or capsule .env files.
        let global_root = match AstridHome::resolve() {
            Ok(home) => Some(home.shared_dir()),
            Err(e) => {
                tracing::warn!(
                    "Could not resolve global Astrid home, global:// will be unavailable: {e}"
                );
                None
            },
        };

        // 1. Initialize MCP process manager
        let mcp_config = ServersConfig::load_default().unwrap_or_default();
        let mcp_manager = ServerManager::new(mcp_config);
        let mcp_client = McpClient::new(mcp_manager);

        // 1. Establish the physical security boundary (sandbox handle)
        let root_handle = DirHandle::new();

        // 2. Initialize the physical filesystem layers
        let lower_vfs = HostVfs::new();
        lower_vfs
            .register_dir(root_handle.clone(), workspace_root.clone())
            .await
            .map_err(|_| std::io::Error::other("Failed to register lower vfs dir"))?;

        let upper_vfs = HostVfs::new();
        upper_vfs
            .register_dir(root_handle.clone(), workspace_root.clone())
            .await
            .map_err(|_| std::io::Error::other("Failed to register upper vfs dir"))?;

        // 3. Wrap in copy-on-write OverlayVfs
        let overlay_vfs = OverlayVfs::new(Box::new(lower_vfs), Box::new(upper_vfs));

        // 4. Bind the secure Unix socket natively
        let listener = socket::bind_session_socket()?;

        let kv = Arc::new(astrid_storage::MemoryKvStore::new());

        let kernel = Arc::new(Self {
            session_id,
            event_bus,
            capsules,
            mcp_client,
            vfs: Arc::new(overlay_vfs),
            vfs_root_handle: root_handle,
            workspace_root,
            global_root,
            cli_socket_listener: Some(Arc::new(tokio::sync::Mutex::new(listener))),
            kv,
        });

        drop(kernel_router::spawn_kernel_router(Arc::clone(&kernel)));
        drop(spawn_idle_monitor(Arc::clone(&kernel)));

        // Spawn the event dispatcher — routes EventBus events to capsule interceptors
        let dispatcher = astrid_capsule::dispatcher::EventDispatcher::new(
            Arc::clone(&kernel.capsules),
            Arc::clone(&kernel.event_bus),
        );
        tokio::spawn(dispatcher.run());

        Ok(kernel)
    }

    /// Load a capsule into the Kernel from a directory containing a Capsule.toml
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be loaded, the capsule cannot be created, or registration fails.
    pub async fn load_capsule(&self, dir: PathBuf) -> Result<(), anyhow::Error> {
        let manifest_path = dir.join("Capsule.toml");
        let manifest = astrid_capsule::discovery::load_manifest(&manifest_path)
            .map_err(|e| anyhow::anyhow!(e))?;

        let loader = astrid_capsule::loader::CapsuleLoader::new(self.mcp_client.clone());
        let mut capsule = loader.create_capsule(manifest, dir.clone())?;

        // Build the context — use the shared kernel KV so capsules can
        // communicate state through overlapping KV namespaces.
        let kv = astrid_storage::ScopedKvStore::new(
            Arc::clone(&self.kv) as Arc<dyn astrid_storage::KvStore>,
            format!("capsule:{}", capsule.id()),
        )?;

        // Pre-load `.env.json` into the KV store if it exists
        let env_path = dir.join(".env.json");
        if env_path.exists()
            && let Ok(contents) = std::fs::read_to_string(&env_path)
            && let Ok(env_map) =
                serde_json::from_str::<std::collections::HashMap<String, String>>(&contents)
        {
            for (k, v) in env_map {
                let _ = kv.set(&k, v.into_bytes()).await;
            }
        }

        let ctx = astrid_capsule::context::CapsuleContext::new(
            self.workspace_root.clone(),
            self.global_root.clone(),
            kv,
            Arc::clone(&self.event_bus),
            self.cli_socket_listener.clone(),
        )
        .with_registry(Arc::clone(&self.capsules));

        capsule.load(&ctx).await?;

        let mut registry = self.capsules.write().await;
        registry
            .register(capsule)
            .map_err(|e| anyhow::anyhow!("Failed to register capsule: {e}"))?;

        Ok(())
    }

    /// Auto-discover and load all capsules from the standard directories (`~/.astrid/capsules` and `.astrid/capsules`).
    ///
    /// Uplink/daemon capsules are loaded first so their event bus subscriptions
    /// are active before other capsules emit events (e.g. `OnboardingRequired`).
    pub async fn load_all_capsules(&self) {
        use astrid_core::dirs::AstridHome;

        let mut paths = Vec::new();
        if let Ok(home) = AstridHome::resolve() {
            paths.push(home.capsules_dir());
        }

        let discovered = astrid_capsule::discovery::discover_manifests(Some(&paths));

        // Partition: uplink/daemon capsules first, then the rest.
        let (uplinks, others): (Vec<_>, Vec<_>) = discovered
            .into_iter()
            .partition(|(m, _)| m.capabilities.uplink);

        // Load uplinks first so their event bus subscriptions are ready.
        for (manifest, dir) in &uplinks {
            if let Err(e) = self.load_capsule(dir.clone()).await {
                tracing::warn!(
                    capsule = %manifest.package.name,
                    error = %e,
                    "Failed to load uplink capsule during discovery"
                );
            }
        }

        // Brief yield to let spawned background `run()` tasks initialize
        // their event bus subscriptions before we load capsules that may
        // emit events like OnboardingRequired.
        if !uplinks.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        for (manifest, dir) in &others {
            if let Err(e) = self.load_capsule(dir.clone()).await {
                tracing::warn!(
                    capsule = %manifest.package.name,
                    error = %e,
                    "Failed to load capsule during discovery"
                );
            }
        }

        // Signal that all capsules have been loaded so uplink capsules
        // (like the registry) can proceed with discovery instead of
        // polling with arbitrary timeouts.
        let msg = astrid_events::ipc::IpcMessage::new(
            "kernel.capsules_loaded",
            astrid_events::ipc::IpcPayload::RawJson(serde_json::json!({"status": "ready"})),
            self.session_id.0,
        );
        let _ = self.event_bus.publish(astrid_events::AstridEvent::Ipc {
            metadata: astrid_events::EventMetadata::new("kernel"),
            message: msg,
        });
    }
}

/// Spawns a background task that cleanly shuts down the Kernel if there is no activity.
///
/// In the current "appable" iteration of the OS, this prevents the background daemon from
/// running forever when the user closes their CLI window.
/// In future iterations (like macOS menubar or unikernel), this monitor can be feature-gated.
fn spawn_idle_monitor(kernel: Arc<Kernel>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Give the OS a grace period to start up and allow clients to connect.
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

            // 1. Are there any active connections to the global socket?
            // Since we handed the listener lock to the proxy capsule, we can't easily
            // query active TCP connections without exposing a status API from the proxy.
            // But we CAN check the Event Bus subscriber count!
            let active_subscribers = kernel.event_bus.subscriber_count();

            // 2. Are there any cron jobs or daemonize capabilities registered?
            let has_daemons = {
                let reg = kernel.capsules.read().await;
                reg.values().any(|c| {
                    let manifest = c.manifest();
                    // If a capsule explicitly acts as an uplink or has cron jobs,
                    // the OS must stay alive to serve them.
                    !manifest.uplinks.is_empty() || !manifest.cron_jobs.is_empty()
                })
            };

            // If there is only 1 subscriber (the internal KernelRouter) and no daemons,
            // the OS is completely dormant.
            if active_subscribers <= 1 && !has_daemons {
                tracing::debug!(
                    "Astrid daemon idle with no active sessions or daemons (auto-shutdown disabled — see FIXME above)"
                );

                // FIXME: The CLI capsule's event bus subscription count
                // is not yet visible to this heuristic, so the idle monitor may
                // fire prematurely. Disabled until the proxy bridge properly
                // registers subscribers.
                // let socket_path = crate::socket::kernel_socket_path();
                // let _ = std::fs::remove_file(&socket_path);
                // std::process::exit(0);
            }
        }
    })
}
