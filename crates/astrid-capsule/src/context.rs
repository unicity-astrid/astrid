//! Capsule context types.
//!
//! Provides the execution context for capsule lifecycle and tool invocations.

use std::path::PathBuf;
use std::sync::Arc;

use astrid_core::SessionId;
use astrid_core::principal::PrincipalId;
use astrid_events::EventBus;
use astrid_storage::ScopedKvStore;
use uuid::Uuid;

use astrid_core::session_token::SessionToken;

use crate::capsule::CapsuleId;
use crate::registry::CapsuleRegistry;

/// Context provided to a capsule during lifecycle operations (load/unload).
///
/// Not `Clone` by design - `session_token` holds secret bytes that should
/// not be accidentally duplicated. Use `Arc<SessionToken>` for cheap sharing.
/// Constructed via `new()` + builder methods (`with_session_token`, etc.).
pub struct CapsuleContext {
    /// The principal this capsule is running on behalf of.
    pub principal: PrincipalId,
    pub workspace_root: PathBuf,
    /// Global shared resources directory (`~/.astrid/home/{principal}/`).
    /// When set, capsules declaring `fs_read = ["global://"]` can read files
    /// under this root via the `global://` path prefix. This is scoped to the
    /// principal's home — keys, databases, and system config in `~/.astrid/`
    /// are NOT accessible through this path.
    pub global_root: Option<PathBuf>,
    pub kv: ScopedKvStore,
    pub event_bus: Arc<EventBus>,
    pub cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    /// Shared capsule registry for `hooks::trigger` fan-out.
    ///
    /// When set, WASM capsules can dispatch hooks to other capsules via
    /// the `astrid_trigger_hook` host function (the kernel mechanism).
    pub capsule_registry: Option<Arc<tokio::sync::RwLock<CapsuleRegistry>>>,
    /// Session token for authenticating CLI socket connections. Only set for
    /// capsules with `net_bind` capability (the CLI proxy capsule).
    pub session_token: Option<Arc<SessionToken>>,
    /// Shared allowance store for capsule-level approval requests.
    pub allowance_store: Option<Arc<astrid_approval::AllowanceStore>>,
    /// Shared identity store for resolving platform users to `AstridUserId`.
    pub identity_store: Option<Arc<dyn astrid_storage::IdentityStore>>,
}

impl CapsuleContext {
    #[must_use]
    pub fn new(
        principal: PrincipalId,
        workspace_root: PathBuf,
        global_root: Option<PathBuf>,
        kv: ScopedKvStore,
        event_bus: Arc<EventBus>,
        cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    ) -> Self {
        Self {
            principal,
            workspace_root,
            global_root,
            kv,
            event_bus,
            cli_socket_listener,
            capsule_registry: None,
            session_token: None,
            allowance_store: None,
            identity_store: None,
        }
    }

    /// Set the session token for socket authentication.
    #[must_use]
    pub fn with_session_token(mut self, token: Arc<SessionToken>) -> Self {
        self.session_token = Some(token);
        self
    }

    /// Set the capsule registry for hook dispatch.
    #[must_use]
    pub fn with_registry(mut self, registry: Arc<tokio::sync::RwLock<CapsuleRegistry>>) -> Self {
        self.capsule_registry = Some(registry);
        self
    }

    /// Set the shared allowance store for capsule-level approval.
    #[must_use]
    pub fn with_allowance_store(mut self, store: Arc<astrid_approval::AllowanceStore>) -> Self {
        self.allowance_store = Some(store);
        self
    }

    /// Set the shared identity store for platform user resolution.
    #[must_use]
    pub fn with_identity_store(mut self, store: Arc<dyn astrid_storage::IdentityStore>) -> Self {
        self.identity_store = Some(store);
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
