//! `SurrealDB` query engine interface.
//!
//! The [`Database`] struct wraps a `SurrealDB` connection and provides
//! typed access for system stores. In embedded mode it uses `SurrealKV`
//! as its storage engine; in distributed mode it uses `TiKV`.
//!
//! # Connection Strings
//!
//! | Mode | Connection | Backend |
//! |------|-----------|---------  |
//! | Embedded (dev) | `surrealkv://path/to/data` | `SurrealKV` |
//! | Embedded (test) | `mem://` | In-memory |
//! | Distributed | `tikv://pd0:2379` | `TiKV` cluster |
//!
//! # Usage
//!
//! ```rust,ignore
//! use astrid_storage::Database;
//!
//! let db = Database::connect_embedded("path/to/data").await?;
//! // or
//! let db = Database::connect_memory().await?;
//! ```

use crate::error::{StorageError, StorageResult};

/// Re-export `SurrealDB` for direct query access when needed.
pub use surrealdb;

/// `SurrealDB` query engine wrapper.
///
/// Provides typed access to the full `SurrealDB` feature set:
/// document storage, graph traversal, relations, `SurrealQL` queries,
/// computed fields, events, and permissions.
pub struct Database {
    inner: surrealdb::Surreal<surrealdb::engine::any::Any>,
}

impl Database {
    /// Connect to an embedded `SurrealDB` with `SurrealKV` storage.
    ///
    /// Data is persisted to the given directory path.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Connection`] if the connection fails.
    pub async fn connect_embedded(path: &str) -> StorageResult<Self> {
        let endpoint = format!("surrealkv://{path}");
        let db: surrealdb::Surreal<surrealdb::engine::any::Any> = surrealdb::Surreal::init();
        db.connect(&endpoint)
            .await
            .map_err(|e: surrealdb::Error| StorageError::Connection(e.to_string()))?;
        db.use_ns("astrid")
            .use_db("main")
            .await
            .map_err(|e: surrealdb::Error| StorageError::Connection(e.to_string()))?;
        Ok(Self { inner: db })
    }

    /// Connect to an in-memory `SurrealDB` (for tests).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Connection`] if the connection fails.
    pub async fn connect_memory() -> StorageResult<Self> {
        let db: surrealdb::Surreal<surrealdb::engine::any::Any> = surrealdb::Surreal::init();
        db.connect("mem://")
            .await
            .map_err(|e: surrealdb::Error| StorageError::Connection(e.to_string()))?;
        db.use_ns("astrid")
            .use_db("test")
            .await
            .map_err(|e: surrealdb::Error| StorageError::Connection(e.to_string()))?;
        Ok(Self { inner: db })
    }

    /// Get a reference to the underlying `SurrealDB` client.
    ///
    /// Use this for direct `SurrealQL` queries when the typed API is
    /// not sufficient.
    #[must_use]
    pub fn client(&self) -> &surrealdb::Surreal<surrealdb::engine::any::Any> {
        &self.inner
    }
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}
