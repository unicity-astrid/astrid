use std::path::PathBuf;
use async_trait::async_trait;
use tracing::info;

use crate::error::CapsuleResult;
use crate::manifest::CapsuleManifest;
use super::ExecutionEngine;

/// Executes Pure WASM Components and AstridClaw transpiled OpenClaw plugins.
///
/// This engine sandboxes the execution in Extism/Wasmtime and injects the
/// `astrid-sys` Airlocks (host functions) so the component can interact
/// securely with the OS Event Bus and VFS.
pub struct WasmEngine {
    manifest: CapsuleManifest,
    _capsule_dir: PathBuf,
}

impl WasmEngine {
    pub fn new(manifest: CapsuleManifest, capsule_dir: PathBuf) -> Self {
        Self {
            manifest,
            _capsule_dir: capsule_dir,
        }
    }
}

#[async_trait]
impl ExecutionEngine for WasmEngine {
    async fn load(&mut self) -> CapsuleResult<()> {
        info!(
            capsule = %self.manifest.package.name,
            "Loading Pure WASM component"
        );

        // TODO: In Phase 5, read the `.wasm` file from `capsule_dir`,
        // instantiate the Extism Plugin, register the `astrid_` host functions,
        // and link the WASM linear memory safely.

        Ok(())
    }

    async fn unload(&mut self) -> CapsuleResult<()> {
        info!(
            capsule = %self.manifest.package.name,
            "Unloading WASM component"
        );
        // Cleanly drop the WASM module and free linear memory
        Ok(())
    }
}