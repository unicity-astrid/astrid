//! Capsule trait and core types.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::CapsuleManifest;

/// Unique, stable, human-readable capsule identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct CapsuleId(String);

impl<'de> Deserialize<'de> for CapsuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl CapsuleId {
    pub fn new(id: impl Into<String>) -> CapsuleResult<Self> {
        let id = id.into();
        Self::validate(&id)?;
        Ok(Self(id))
    }

    #[must_use]
    pub fn from_static(id: &str) -> Self {
        Self(id.to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(id: &str) -> CapsuleResult<()> {
        if id.is_empty() {
            return Err(CapsuleError::UnsupportedEntryPoint(
                "capsule id must not be empty".into(),
            ));
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "capsule id must contain only lowercase alphanumeric characters and hyphens, got: {id}"
            )));
        }
        Ok(())
    }
}

impl fmt::Display for CapsuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CapsuleId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The lifecycle state of a capsule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsuleState {
    Unloaded,
    Loading,
    Ready,
    Failed(String),
    Unloading,
}

/// A loaded capsule that can provide tools and integrations to the runtime.
#[async_trait]
pub trait Capsule: Send + Sync {
    /// The unique identifier for this capsule.
    fn id(&self) -> &CapsuleId;

    /// The manifest that describes this capsule.
    fn manifest(&self) -> &CapsuleManifest;

    /// Current lifecycle state.
    fn state(&self) -> CapsuleState;

    /// Load the capsule, initializing all of its execution engines.
    async fn load(&mut self) -> CapsuleResult<()>;

    /// Unload the capsule, terminating all of its execution engines.
    async fn unload(&mut self) -> CapsuleResult<()>;
}

/// The universal, additive implementation of a Capsule.
///
/// Instead of choosing between WASM or MCP execution, the `CompositeCapsule`
/// owns a collection of `ExecutionEngine`s. When loaded, it iterates through
/// all of them, providing a unified lifecycle and security boundary for
/// everything declared in the `Capsule.toml`.
pub struct CompositeCapsule {
    id: CapsuleId,
    manifest: CapsuleManifest,
    state: CapsuleState,
    engines: Vec<Box<dyn crate::engine::ExecutionEngine>>,
}

impl CompositeCapsule {
    /// Create a new, empty Composite Capsule from a manifest.
    pub fn new(manifest: CapsuleManifest) -> CapsuleResult<Self> {
        let id = CapsuleId::new(manifest.package.name.clone())?;
        Ok(Self {
            id,
            manifest,
            state: CapsuleState::Unloaded,
            engines: Vec::new(),
        })
    }

    /// Add an execution engine (e.g., WasmEngine, McpEngine) to this capsule.
    pub fn add_engine(&mut self, engine: Box<dyn crate::engine::ExecutionEngine>) {
        self.engines.push(engine);
    }
}

#[async_trait]
impl Capsule for CompositeCapsule {
    fn id(&self) -> &CapsuleId {
        &self.id
    }

    fn manifest(&self) -> &CapsuleManifest {
        &self.manifest
    }

    fn state(&self) -> CapsuleState {
        self.state.clone()
    }

    async fn load(&mut self) -> CapsuleResult<()> {
        self.state = CapsuleState::Loading;
        for engine in &mut self.engines {
            if let Err(e) = engine.load().await {
                self.state = CapsuleState::Failed(e.to_string());
                return Err(e);
            }
        }
        self.state = CapsuleState::Ready;
        Ok(())
    }

    async fn unload(&mut self) -> CapsuleResult<()> {
        self.state = CapsuleState::Unloading;
        for engine in &mut self.engines {
            // Unload on a best-effort basis so a failing engine doesn't
            // prevent others from shutting down gracefully.
            let _ = engine.unload().await;
        }
        self.state = CapsuleState::Unloaded;
        Ok(())
    }
}
