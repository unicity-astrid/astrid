//! Daemon `WebSocket` server.
//!
//! Implements the `jsonrpsee` server that listens on `127.0.0.1:{port}` and serves
//! the [`AstridRpc`] API. CLI clients connect via `WebSocket`.
//!
//! # Locking Design
//!
//! The runtime is stored behind a standalone `Arc` (immutable reference, never locked).
//! Sessions live in per-session `Mutex<AgentSession>` behind a shared session map.
//! The session map itself uses an `RwLock` but only for brief insert/remove/lookup —
//! never held across async operations like LLM calls or approval waits.
//!
//! This prevents the deadlock where `send_input` (holding a write lock during an
//! LLM turn) blocks `approval_response` (needing a read lock to deliver the
//! approval that the turn is waiting for).

mod inbound_router;
mod lifecycle;
mod monitoring;
mod paths;
mod plugins;
mod rpc;
mod startup;

pub use paths::DaemonPaths;
pub use startup::DaemonStartOptions;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use astrid_core::InboundMessage;
use astrid_core::SessionId;
use astrid_core::identity::IdentityStore;
use astrid_llm::LlmProvider;
use astrid_mcp::McpClient;
use astrid_plugins::{PluginId, PluginRegistry, WasmPluginLoader};
use astrid_runtime::{AgentRuntime, AgentSession};
use astrid_storage::KvStore;
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use uuid::Uuid;

use crate::daemon_frontend::DaemonFrontend;
use crate::rpc::DaemonEvent;

/// Handle to a live session's shared state.
///
/// All fields are `Arc`-wrapped so `SessionHandle` is cheaply cloneable.
/// The `AgentSession` is behind a per-session `Mutex` so each session can
/// run independently without blocking the entire daemon.
#[derive(Clone)]
struct SessionHandle {
    /// The agent session (per-session lock — only locked during a turn).
    session: Arc<Mutex<AgentSession>>,
    /// The daemon frontend for this session (bridges Frontend trait to IPC).
    frontend: Arc<DaemonFrontend>,
    /// Broadcast channel for events going to CLI subscribers.
    event_tx: broadcast::Sender<DaemonEvent>,
    /// The workspace path for this session (if any).
    workspace: Option<PathBuf>,
    /// When the session was created (immutable).
    created_at: DateTime<Utc>,
    /// Handle to the currently running turn task (if any).
    turn_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Canonical Astrid user ID bound to this session.
    ///
    /// `None` for CLI-originated sessions (addressed by `SessionId` via RPC).
    /// `Some` for connector-originated sessions created by the inbound router.
    /// Read by future RPC endpoints that expose per-user session info.
    #[allow(dead_code)]
    user_id: Option<Uuid>,
}

/// The daemon `WebSocket` server.
pub struct DaemonServer {
    /// The agent runtime (shared, immutable reference).
    runtime: Arc<AgentRuntime<Box<dyn LlmProvider>>>,
    /// Session map (brief locks only for insert/remove/lookup).
    sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    /// Plugin registry (shared across RPC handlers).
    plugin_registry: Arc<RwLock<PluginRegistry>>,
    /// Workspace KV store (used for plugin scoped storage on reload).
    workspace_kv: Arc<dyn KvStore>,
    /// MCP client (used to re-create MCP plugins on reload).
    mcp_client: McpClient,
    /// WASM plugin loader (shared configuration for reload consistency).
    wasm_loader: Arc<WasmPluginLoader>,
    /// Home directory for plugin paths.
    home: astrid_core::dirs::AstridHome,
    /// Workspace root directory.
    workspace_root: PathBuf,
    /// When the daemon started.
    #[allow(dead_code)]
    started_at: Instant,
    /// Shutdown signal.
    shutdown_tx: broadcast::Sender<()>,
    /// Filesystem paths for PID/port files.
    paths: DaemonPaths,
    /// Interval between health checks (from config, floored at 5s).
    health_interval: Duration,
    /// Whether this daemon is running in ephemeral mode.
    ephemeral: bool,
    /// Grace period before ephemeral shutdown (seconds).
    ephemeral_grace_secs: u64,
    /// Number of active `WebSocket` connections (event subscribers).
    active_connections: Arc<AtomicUsize>,
    /// Interval between session cleanup sweeps.
    session_cleanup_interval: Duration,
    /// Plugin IDs explicitly unloaded by the user via RPC.
    ///
    /// The watcher skips these to avoid re-loading plugins the user
    /// intentionally stopped. Cleared when the user re-loads via RPC.
    user_unloaded_plugins: Arc<RwLock<HashSet<PluginId>>>,
    /// Identity store for resolving platform users to canonical Astrid identities.
    /// Stored here for future RPC endpoints (e.g. list/link identities).
    #[allow(dead_code)]
    identity_store: Arc<dyn IdentityStore>,
    /// Sender side of the central inbound message channel.
    ///
    /// Cloned for each plugin that declares connector capability. The inbound
    /// router task holds the receiver end and drains it to route messages.
    inbound_tx: mpsc::Sender<InboundMessage>,
    /// Maps `AstridUserId` (UUID) → most recent active `SessionId`.
    ///
    /// Stored here for future RPC endpoints. The inbound router holds its own
    /// `Arc` clone and manages this map directly.
    #[allow(dead_code)]
    connector_sessions: Arc<RwLock<HashMap<Uuid, SessionId>>>,
}

impl DaemonServer {
    /// Whether this daemon is running in ephemeral mode.
    #[must_use]
    pub fn is_ephemeral(&self) -> bool {
        self.ephemeral
    }

    /// Subscribe to the shutdown signal.
    ///
    /// The returned receiver fires when an RPC `shutdown()` call is made.
    /// Use with `tokio::select!` alongside `ctrl_c()` in the daemon's
    /// foreground loop.
    #[must_use]
    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Read the port from the port file (used by CLI to find the daemon).
    #[must_use]
    pub fn read_port(paths: &DaemonPaths) -> Option<u16> {
        std::fs::read_to_string(paths.port_file())
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    /// Read the PID from the PID file.
    #[must_use]
    pub fn read_pid(paths: &DaemonPaths) -> Option<u32> {
        std::fs::read_to_string(paths.pid_file())
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    /// Check if a daemon is running (PID file exists and process is alive).
    #[must_use]
    pub fn is_running(paths: &DaemonPaths) -> bool {
        if let Some(pid) = Self::read_pid(paths) {
            is_process_alive(pid)
        } else {
            false
        }
    }
}

/// Check if a process with the given PID is alive.
fn is_process_alive(pid: u32) -> bool {
    // Use `kill -0 <pid>` to check if the process exists.
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
