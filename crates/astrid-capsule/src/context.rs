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

/// Context provided to a capsule during lifecycle operations (load/unload).
#[derive(Debug, Clone)]
pub struct CapsuleContext {
    pub workspace_root: PathBuf,
    pub kv: ScopedKvStore,
    pub event_bus: Arc<EventBus>,
}

impl CapsuleContext {
    #[must_use]
    pub fn new(workspace_root: PathBuf, kv: ScopedKvStore, event_bus: Arc<EventBus>) -> Self {
        Self {
            workspace_root,
            kv,
            event_bus,
        }
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
