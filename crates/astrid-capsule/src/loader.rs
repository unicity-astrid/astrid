//! Factory and routing logic for instantiating Composite Capsules.

use std::path::PathBuf;

use crate::capsule::{Capsule, CompositeCapsule};
use crate::error::CapsuleResult;
use crate::manifest::CapsuleManifest;

/// Responsible for translating a declarative `Capsule.toml` manifest into
/// a live, unified `CompositeCapsule` packed with the correct execution engines.
pub struct CapsuleLoader {
    // TODO: In Phase 5, this will hold Arc references to the Wasmtime Engine
    // and Security Gates so it can pass them down into the WasmEngine instances.
}

impl CapsuleLoader {
    /// Create a new Capsule Loader.
    #[must_use]
    pub fn new() -> Self {
        Self {}
    }

    /// Parse a `CapsuleManifest` and build a unified `CompositeCapsule`.
    ///
    /// This method is the "router" of the Manifest-First architecture. It inspects
    /// the declarative TOML and provisions the correct runtime environments (WASM,
    /// Host Process, Static Context) securely into a single Capsule object.
    ///
    /// # Errors
    /// Returns a `CapsuleError` if the manifest is invalid or requests an
    /// unsupported engine configuration.
    pub fn create_capsule(
        &self,
        manifest: CapsuleManifest,
        capsule_dir: PathBuf,
    ) -> CapsuleResult<Box<dyn Capsule>> {
        let mut composite = CompositeCapsule::new(manifest.clone())?;

        // 1. WASM Component Engine (Pure WASM or Compiled OpenClaw)
        if manifest.component.is_some() {
            composite.add_engine(Box::new(crate::engine::WasmEngine::new(
                manifest.clone(),
                capsule_dir.clone(),
            )));
        }

        // 2. Legacy Host MCP Engine (The Airlock Override)
        for server in &manifest.mcp_servers {
            // If server.server_type == "stdio", then the user is explicitly requesting
            // a host process breakout.
            if server.server_type.as_deref() == Some("stdio") {
                composite.add_engine(Box::new(crate::engine::McpHostEngine::new(
                    manifest.clone(),
                    server.clone(),
                    capsule_dir.clone(),
                )));
            }
        }
        // 3. Static Context Engine
        // Always added. Handles injecting context_files, static commands, and skills
        // directly into the OS memory without booting any VMs or Processes.
        composite.add_engine(Box::new(crate::engine::StaticEngine::new(
            manifest.clone(),
            capsule_dir,
        )));

        Ok(Box::new(composite))
    }
}

impl Default for CapsuleLoader {
    fn default() -> Self {
        Self::new()
    }
}
