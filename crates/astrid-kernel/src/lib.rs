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

use astrid_audit::AuditLog;
use astrid_capabilities::{CapabilityStore, DirHandle};
use astrid_capsule::registry::CapsuleRegistry;
use astrid_core::SessionId;
use astrid_crypto::KeyPair;
use astrid_events::EventBus;
use astrid_mcp::{McpClient, SecureMcpClient, ServerManager, ServersConfig};
use astrid_vfs::{HostVfs, OverlayVfs, Vfs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;

/// The core Operating System Kernel.
pub struct Kernel {
    /// The unique identifier for this kernel session.
    pub session_id: SessionId,
    /// The global IPC message bus.
    pub event_bus: Arc<EventBus>,
    /// The process manager (loaded WASM capsules).
    pub capsules: Arc<RwLock<CapsuleRegistry>>,
    /// The secure MCP client with capability-based authorization and audit logging.
    pub mcp: SecureMcpClient,
    /// The capability store for this session.
    pub capabilities: Arc<CapabilityStore>,
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
    pub kv: Arc<astrid_storage::SurrealKvStore>,
    /// Chain-linked cryptographic audit log with persistent storage.
    pub audit_log: Arc<AuditLog>,
    /// Number of active client connections (CLI sessions).
    pub active_connections: AtomicUsize,
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

        // Resolve the Astrid home directory. Required for persistent KV store
        // and audit log. Fails boot if neither $ASTRID_HOME nor $HOME is set.
        let home = AstridHome::resolve().map_err(|e| {
            std::io::Error::other(format!(
                "Failed to resolve Astrid home (set $ASTRID_HOME or $HOME): {e}"
            ))
        })?;

        // Resolve the global shared directory for the `global://` VFS scheme.
        // Scoped to `~/.astrid/shared/` — NOT the full `~/.astrid/` root — so
        // capsules cannot access keys, databases, or capsule .env files.
        let global_root = Some(home.shared_dir());

        // 1. Initialize MCP process manager with security layer
        let mcp_config = ServersConfig::load_default().unwrap_or_default();
        let mcp_manager = ServerManager::new(mcp_config);
        let mcp_client = McpClient::new(mcp_manager);

        // 2. Bootstrap capability store and persistent audit log.
        // TODO: Wire CapabilityStore persistence. Currently in-memory only
        // so capability tokens are lost on restart. The runtime signing key
        // is now persisted via load_or_generate_runtime_key(), but a key
        // rotation / migration strategy is needed before persisting tokens
        // (a fresh key invalidates all tokens signed by the old one).
        let capabilities = Arc::new(CapabilityStore::in_memory());
        let audit_log = open_audit_log()?;
        let mcp = SecureMcpClient::new(
            mcp_client,
            Arc::clone(&capabilities),
            Arc::clone(&audit_log),
            session_id.clone(),
        );

        // 3. Establish the physical security boundary (sandbox handle)
        let root_handle = DirHandle::new();

        // 4. Initialize the physical filesystem layers
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

        // 5. Wrap in copy-on-write OverlayVfs
        let overlay_vfs = OverlayVfs::new(Box::new(lower_vfs), Box::new(upper_vfs));

        // 6. Bind the secure Unix socket natively
        let listener = socket::bind_session_socket()?;

        let kv_path = home.state_db_path();
        let kv = Arc::new(
            astrid_storage::SurrealKvStore::open(&kv_path)
                .map_err(|e| std::io::Error::other(format!("Failed to open KV store: {e}")))?,
        );
        // TODO: clear ephemeral keys (e: prefix) on boot when the key
        // lifecycle tier convention is established.

        let kernel = Arc::new(Self {
            session_id,
            event_bus,
            capsules,
            mcp,
            capabilities,
            vfs: Arc::new(overlay_vfs),
            vfs_root_handle: root_handle,
            workspace_root,
            global_root,
            cli_socket_listener: Some(Arc::new(tokio::sync::Mutex::new(listener))),
            kv,
            audit_log,
            active_connections: AtomicUsize::new(0),
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
    async fn load_capsule(&self, dir: PathBuf) -> Result<(), anyhow::Error> {
        let manifest_path = dir.join("Capsule.toml");
        let manifest = astrid_capsule::discovery::load_manifest(&manifest_path)
            .map_err(|e| anyhow::anyhow!(e))?;

        let loader = astrid_capsule::loader::CapsuleLoader::new(self.mcp.clone());
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

    /// Record that a new client connection has been established.
    pub fn connection_opened(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a client connection has been closed.
    ///
    /// Uses `fetch_update` for atomic saturating decrement - avoids the TOCTOU
    /// window where `fetch_sub` wraps to `usize::MAX` before a corrective store.
    pub fn connection_closed(&self) {
        let _ = self
            .active_connections
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                if n == 0 {
                    None
                } else {
                    Some(n.saturating_sub(1))
                }
            });
    }

    /// Number of active client connections.
    pub fn connection_count(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    /// Gracefully shut down the kernel.
    ///
    /// 1. Publish `KernelShutdown` event on the bus.
    /// 2. Drain and unload all capsules (stops MCP child processes, WASM engines).
    /// 3. Flush and close the persistent KV store.
    /// 4. Remove the Unix socket file.
    pub async fn shutdown(&self, reason: Option<String>) {
        tracing::info!(reason = ?reason, "Kernel shutting down");

        // 1. Notify all subscribers so capsules can react.
        let _ = self
            .event_bus
            .publish(astrid_events::AstridEvent::KernelShutdown {
                metadata: astrid_events::EventMetadata::new("kernel"),
                reason: reason.clone(),
            });

        // 2. Drain the registry and unload each capsule.
        let capsules = {
            let mut reg = self.capsules.write().await;
            reg.drain()
        };
        for mut arc in capsules {
            let id = arc.id().clone();
            if let Some(capsule) = Arc::get_mut(&mut arc) {
                if let Err(e) = capsule.unload().await {
                    tracing::warn!(
                        capsule_id = %id,
                        error = %e,
                        "Failed to unload capsule during shutdown"
                    );
                }
            } else {
                tracing::warn!(
                    capsule_id = %id,
                    "Cannot unload capsule: other references still held"
                );
            }
        }

        // 3. Flush the persistent KV store.
        if let Err(e) = self.kv.close().await {
            tracing::warn!(error = %e, "Failed to flush KV store during shutdown");
        }

        // 4. Remove the socket file so stale-socket detection works on next boot.
        let socket_path = crate::socket::kernel_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        tracing::info!("Kernel shutdown complete");
    }
}

/// Open (or create) the persistent audit log and verify historical chain integrity.
///
/// Loads the runtime signing key from `~/.astrid/keys/runtime.key`, generating a
/// new one if it doesn't exist. Opens the `SurrealKV`-backed audit database at
/// `~/.astrid/audit.db` and runs `verify_all()` to detect any tampering of
/// historical entries. Verification failures are logged at `error!` level but
/// do not block boot (fail-open for availability, loud alert for integrity).
fn open_audit_log() -> std::io::Result<Arc<AuditLog>> {
    use astrid_core::dirs::AstridHome;

    let home = AstridHome::resolve()
        .map_err(|e| std::io::Error::other(format!("cannot resolve Astrid home: {e}")))?;
    home.ensure()
        .map_err(|e| std::io::Error::other(format!("cannot create Astrid home dirs: {e}")))?;

    let runtime_key = load_or_generate_runtime_key(&home.keys_dir())?;
    let audit_log = AuditLog::open(home.audit_db_path(), runtime_key)
        .map_err(|e| std::io::Error::other(format!("cannot open audit log: {e}")))?;

    // Verify all historical chains on boot.
    match audit_log.verify_all() {
        Ok(results) => {
            let total_sessions = results.len();
            let mut tampered_sessions: usize = 0;

            for (session_id, result) in &results {
                if !result.valid {
                    tampered_sessions = tampered_sessions.saturating_add(1);
                    for issue in &result.issues {
                        tracing::error!(
                            session_id = %session_id,
                            issue = %issue,
                            "Audit chain integrity violation detected"
                        );
                    }
                }
            }

            if tampered_sessions > 0 {
                tracing::error!(
                    total_sessions,
                    tampered_sessions,
                    "Audit chain verification found tampered sessions"
                );
            } else if total_sessions > 0 {
                tracing::info!(
                    total_sessions,
                    "Audit chain verification passed for all sessions"
                );
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "Audit chain verification failed to run");
        },
    }

    Ok(Arc::new(audit_log))
}

/// Load the runtime ed25519 signing key from disk, or generate and persist a new one.
///
/// The key file is 32 bytes of raw secret key material at `{keys_dir}/runtime.key`.
fn load_or_generate_runtime_key(keys_dir: &Path) -> std::io::Result<KeyPair> {
    let key_path = keys_dir.join("runtime.key");

    if key_path.exists() {
        let bytes = std::fs::read(&key_path)?;
        KeyPair::from_secret_key(&bytes).map_err(|e| {
            std::io::Error::other(format!(
                "invalid runtime key at {}: {e}",
                key_path.display()
            ))
        })
    } else {
        let keypair = KeyPair::generate();
        std::fs::create_dir_all(keys_dir)?;
        std::fs::write(&key_path, keypair.secret_key_bytes())?;

        // Secure permissions (owner-only) on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        tracing::info!(key_id = %keypair.key_id_hex(), "Generated new runtime signing key");
        Ok(keypair)
    }
}

/// Spawns a background task that cleanly shuts down the Kernel if there is no activity.
///
/// Uses dual-signal idle detection:
/// - **Primary:** explicit `active_connections` counter (incremented on first IPC
///   message per source, decremented on `Disconnect`).
/// - **Secondary:** `EventBus::subscriber_count()` minus the kernel router's own
///   subscription. When a CLI process dies without sending `Disconnect`, its
///   broadcast receiver is dropped so the subscriber count falls.
///
/// Takes the minimum of both signals to handle ungraceful disconnects.
///
/// Configurable via `ASTRID_IDLE_TIMEOUT_SECS` (default 300 = 5 minutes).
/// Number of permanent internal event bus subscribers that are not client
/// connections: `KernelRouter` (`kernel.request.*`), `ConnectionTracker` (`client.*`),
/// and `EventDispatcher` (all events).
const INTERNAL_SUBSCRIBER_COUNT: usize = 3;

fn spawn_idle_monitor(kernel: Arc<Kernel>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let grace = std::time::Duration::from_secs(30);
        let timeout_secs: u64 = std::env::var("ASTRID_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);
        let idle_timeout = std::time::Duration::from_secs(timeout_secs);
        let check_interval = std::time::Duration::from_secs(15);

        tokio::time::sleep(grace).await;
        let mut idle_since: Option<tokio::time::Instant> = None;

        loop {
            tokio::time::sleep(check_interval).await;

            let connections = kernel.connection_count();

            // Secondary signal: broadcast subscriber count. Subtract the
            // permanent internal subscribers: KernelRouter (kernel.request.*),
            // ConnectionTracker (client.*), and EventDispatcher (all events).
            let bus_subscribers = kernel
                .event_bus
                .subscriber_count()
                .saturating_sub(INTERNAL_SUBSCRIBER_COUNT);

            // Take the minimum: if a CLI died without Disconnect, the counter
            // stays inflated but the subscriber count drops.
            let effective_connections = connections.min(bus_subscribers);

            let has_daemons = {
                let reg = kernel.capsules.read().await;
                reg.values().any(|c| {
                    let m = c.manifest();
                    !m.uplinks.is_empty() || !m.cron_jobs.is_empty()
                })
            };

            if effective_connections == 0 && !has_daemons {
                let now = tokio::time::Instant::now();
                let start = *idle_since.get_or_insert(now);
                let elapsed = now.duration_since(start);

                tracing::debug!(
                    idle_secs = elapsed.as_secs(),
                    timeout_secs,
                    connections,
                    bus_subscribers,
                    "Kernel idle, monitoring timeout"
                );

                if elapsed >= idle_timeout {
                    tracing::info!("Idle timeout reached, initiating shutdown");
                    kernel.shutdown(Some("idle_timeout".to_string())).await;
                    std::process::exit(0);
                }
            } else {
                if idle_since.is_some() {
                    tracing::debug!(
                        effective_connections,
                        has_daemons,
                        "Activity detected, resetting idle timer"
                    );
                }
                idle_since = None;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_or_generate_creates_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");

        let keypair = load_or_generate_runtime_key(&keys_dir).unwrap();
        let key_path = keys_dir.join("runtime.key");

        // Key file should exist with 32 bytes.
        assert!(key_path.exists());
        let bytes = std::fs::read(&key_path).unwrap();
        assert_eq!(bytes.len(), 32);

        // The written bytes should reconstruct the same public key.
        let reloaded = KeyPair::from_secret_key(&bytes).unwrap();
        assert_eq!(
            keypair.public_key_bytes(),
            reloaded.public_key_bytes(),
            "reloaded key should match generated key"
        );
    }

    #[test]
    fn test_load_or_generate_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");

        let first = load_or_generate_runtime_key(&keys_dir).unwrap();
        let second = load_or_generate_runtime_key(&keys_dir).unwrap();

        assert_eq!(
            first.public_key_bytes(),
            second.public_key_bytes(),
            "loading the same key file should produce the same keypair"
        );
    }

    #[test]
    fn test_load_or_generate_rejects_bad_key_length() {
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        std::fs::create_dir_all(&keys_dir).unwrap();

        // Write a key file with wrong length.
        std::fs::write(keys_dir.join("runtime.key"), [0u8; 16]).unwrap();

        let result = load_or_generate_runtime_key(&keys_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid runtime key"),
            "expected 'invalid runtime key' error, got: {err}"
        );
    }

    #[test]
    fn test_connection_counter_increment_decrement() {
        let counter = AtomicUsize::new(0);

        // Simulate connection_opened
        counter.fetch_add(1, Ordering::Relaxed);
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 2);

        // Simulate connection_closed
        counter.fetch_sub(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        counter.fetch_sub(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_connection_counter_underflow_guard() {
        // Test the saturating behavior: decrementing from 0 should stay at 0.
        // Mirrors the fetch_update logic in connection_closed().
        let counter = AtomicUsize::new(0);

        let result = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            if n == 0 { None } else { Some(n - 1) }
        });
        // fetch_update returns Err(0) when the closure returns None (no-op).
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_load_or_generate_sets_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");

        let _ = load_or_generate_runtime_key(&keys_dir).unwrap();

        let key_path = keys_dir.join("runtime.key");
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "key file should have 0o600 permissions, got {mode:#o}"
        );
    }
}
