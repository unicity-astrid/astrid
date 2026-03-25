//! Capsule context types.
//!
//! Provides the execution context for capsule lifecycle and tool invocations.

use std::path::PathBuf;
use std::sync::Arc;

use astrid_core::principal::PrincipalId;
use astrid_events::EventBus;
use astrid_storage::ScopedKvStore;

use astrid_core::session_token::SessionToken;

use crate::registry::CapsuleRegistry;
use crate::schema_catalog::SchemaCatalog;

/// Context provided to a capsule during lifecycle operations (load/unload).
///
/// Not `Clone` by design - `session_token` holds secret bytes that should
/// not be accidentally duplicated. Use `Arc<SessionToken>` for cheap sharing.
/// Constructed via `new()` + builder methods (`with_session_token`, etc.).
pub struct CapsuleContext {
    /// The principal this capsule is running on behalf of.
    pub principal: PrincipalId,
    pub workspace_root: PathBuf,
    /// Home resources directory (`~/.astrid/home/{principal}/`).
    /// When set, capsules declaring `fs_read = ["home://"]` can read files
    /// under this root via the `home://` path prefix. This is scoped to the
    /// principal's home — keys, databases, and system config in `~/.astrid/`
    /// are NOT accessible through this path.
    pub home_root: Option<PathBuf>,
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
    /// Shared schema catalog for topic→schema mappings (A2UI Track 2).
    ///
    /// Updated on capsule load/unload. The A2UI bridge reads this to generate
    /// schema context for the LLM system prompt.
    pub schema_catalog: Arc<SchemaCatalog>,
}

impl CapsuleContext {
    #[must_use]
    pub fn new(
        principal: PrincipalId,
        workspace_root: PathBuf,
        home_root: Option<PathBuf>,
        kv: ScopedKvStore,
        event_bus: Arc<EventBus>,
        cli_socket_listener: Option<Arc<tokio::sync::Mutex<tokio::net::UnixListener>>>,
    ) -> Self {
        Self {
            principal,
            workspace_root,
            home_root,
            kv,
            event_bus,
            cli_socket_listener,
            capsule_registry: None,
            session_token: None,
            allowance_store: None,
            identity_store: None,
            schema_catalog: Arc::new(SchemaCatalog::new()),
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
