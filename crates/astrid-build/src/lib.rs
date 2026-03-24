//! Astrid Build - Capsule compilation and packaging tool.
//!
//! Compiles Rust, `OpenClaw`, and legacy MCP projects into `.capsule` archives.
//! Typically invoked by the CLI (`astrid build`) but can be used standalone.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod archiver;
mod build;
mod mcp;
mod openclaw;
mod rust;

/// Run the build tool with the given arguments.
///
/// This is the library entry point used by both the standalone `astrid-build`
/// binary and the bundled `astrid` CLI.
///
/// # Errors
/// Returns an error if the build fails (missing manifest, compile error, etc.).
pub fn run(
    path: Option<&str>,
    output: Option<&str>,
    project_type: Option<&str>,
    from_mcp_json: Option<&str>,
) -> anyhow::Result<()> {
    build::run_build(path, output, project_type, from_mcp_json)
}

/// Run the internal Wizer subprocess for `OpenClaw` compilation.
///
/// This is a hidden internal entry point used by the compiler subprocess.
///
/// # Errors
/// Returns an error if Wizer fails to produce the output WASM.
pub fn run_wizer_internal(output: &std::path::Path) -> anyhow::Result<()> {
    astrid_openclaw::compiler::run_wizer_internal(output)
        .map_err(|e| anyhow::anyhow!("wizer-internal failed: {e}"))
}
