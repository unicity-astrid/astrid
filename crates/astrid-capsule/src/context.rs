//! Capsule context types.
//!
//! Provides the execution context for capsule lifecycle and tool invocations.

use std::path::PathBuf;
use std::sync::Arc;

use astrid_core::SessionId;
use astrid_events::EventBus;
use astrid_storage::ScopedKvStore;
use uuid::Uuid;

use crate::capsule::CapsuleId;
use crate::registry::CapsuleRegistry;

/// Context provided to a capsule during lifecycle operations (load/unload).
#[derive(Clone)]
pub struct CapsuleContext {
    pub workspace_root: PathBuf,
    /// Global shared resources directory (`~/.astrid/shared/`). When set,
    /// capsules declaring `fs_read = ["global://"]` can read files under
    /// this root via the `global://` path prefix. This is scoped to the
    /// `shared/` subdirectory — keys, databases, and capsule secrets in
    /// `~/.astrid/` are NOT accessible through this path.
    pub global_root: Option<PathBuf>,
    pub kv: ScopedKvStore,
    pub event_bus: Arc<EventBus>,
    pub cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    /// Shared capsule registry for `hooks::trigger` fan-out.
    ///
    /// When set, WASM capsules can dispatch hooks to other capsules via
    /// the `astrid_trigger_hook` host function (the kernel mechanism).
    pub capsule_registry: Option<Arc<tokio::sync::RwLock<CapsuleRegistry>>>,
}

impl CapsuleContext {
    #[must_use]
    pub fn new(
        workspace_root: PathBuf,
        global_root: Option<PathBuf>,
        kv: ScopedKvStore,
        event_bus: Arc<EventBus>,
        cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    ) -> Self {
        Self {
            workspace_root,
            global_root,
            kv,
            event_bus,
            cli_socket_listener,
            capsule_registry: None,
        }
    }

    /// Set the capsule registry for hook dispatch.
    #[must_use]
    pub fn with_registry(mut self, registry: Arc<tokio::sync::RwLock<CapsuleRegistry>>) -> Self {
        self.capsule_registry = Some(registry);
        self
    }
}

/// Context provided to a capsule tool during execution.
#[derive(Debug, Clone)]
pub struct CapsuleToolContext {
    pub capsule_id: CapsuleId,
    pub workspace_root: PathBuf,
    pub kv: ScopedKvStore,
    pub session_id: Option<SessionId>,
    pub user_id: Option<Uuid>,
}

impl CapsuleToolContext {
    #[must_use]
    pub fn new(capsule_id: CapsuleId, workspace_root: PathBuf, kv: ScopedKvStore) -> Self {
        Self {
            capsule_id,
            workspace_root,
            kv,
            session_id: None,
            user_id: None,
        }
    }

    #[must_use]
    pub fn with_session(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    #[must_use]
    pub fn with_user(mut self, user_id: Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }
}
