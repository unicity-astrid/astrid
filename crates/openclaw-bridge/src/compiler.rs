//! `extism-js` invocation for compiling JS → WASM.
//!
//! Shells out to `extism-js` which uses `QuickJS` to compile `JavaScript`
//! into a WASM module compatible with the Extism plugin system.

use std::path::Path;
use std::process::Command;

use crate::error::{BridgeError, BridgeResult};

/// Compile a JS file to a WASM module via `extism-js`.
///
/// The output WASM file is written to `output_path`.
///
/// # Errors
///
/// Returns `BridgeError::ToolNotFound` if `extism-js` is not in PATH, or
/// `BridgeError::CompileFailed` if the compilation process fails.
pub fn compile(js_path: &Path, output_path: &Path) -> BridgeResult<()> {
    // Check that extism-js is installed
    which::which("extism-js").map_err(|_| BridgeError::ToolNotFound {
        tool: "extism-js".into(),
        install_hint: "https://github.com/nicholasgasior/extism-js/releases".into(),
    })?;

    let output = Command::new("extism-js")
        .arg(js_path)
        .arg("-o")
        .arg(output_path)
        .output()
        .map_err(|e| BridgeError::CompileFailed(format!("failed to run extism-js: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::CompileFailed(format!(
            "extism-js exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    // Verify the output file was actually created
    if !output_path.exists() {
        return Err(BridgeError::CompileFailed(
            "extism-js succeeded but output file was not created".into(),
        ));
    }

    Ok(())
}
