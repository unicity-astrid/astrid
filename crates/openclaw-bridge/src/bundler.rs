//! `esbuild` invocation for `TypeScript` transpilation and multi-file bundling.
//!
//! Shells out to `esbuild` with `--bundle --format=cjs --target=es2020 --platform=neutral`.
//! The CJS format is required because `QuickJS` (used by `extism-js`) requires CJS modules.

use std::path::Path;
use std::process::Command;

use crate::error::{BridgeError, BridgeResult};

/// Bundle a JS/TS entry point into a self-contained CJS module via esbuild.
///
/// Returns the bundled source code as a string.
///
/// # Errors
///
/// Returns `BridgeError::ToolNotFound` if esbuild is not in PATH, or
/// `BridgeError::BundleFailed` if the bundling process fails.
pub fn bundle(entry_point: &Path) -> BridgeResult<String> {
    // Check that esbuild is installed
    which::which("esbuild").map_err(|_| BridgeError::ToolNotFound {
        tool: "esbuild".into(),
        install_hint: "npm i -g esbuild".into(),
    })?;

    let output = Command::new("esbuild")
        .arg(entry_point)
        .arg("--bundle")
        .arg("--format=cjs")
        .arg("--target=es2020")
        .arg("--platform=neutral")
        .output()
        .map_err(|e| BridgeError::BundleFailed(format!("failed to run esbuild: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::BundleFailed(format!(
            "esbuild exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| BridgeError::BundleFailed(format!("esbuild output is not valid UTF-8: {e}")))
}
