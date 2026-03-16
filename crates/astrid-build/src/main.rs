//! Astrid Build - Capsule compilation and packaging tool.
//!
//! Compiles Rust, OpenClaw, and legacy MCP projects into `.capsule` archives.
//! Typically invoked by the CLI (`astrid build`) but can be used standalone.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use anyhow::Result;
use clap::Parser;

mod archiver;
mod build;

/// Astrid Build - Capsule compilation and packaging
#[derive(Parser)]
#[command(name = "astrid-build")]
#[command(author, version, about)]
struct Args {
    /// Path to the project directory (defaults to current directory)
    path: Option<String>,

    /// Output directory for the packaged `.capsule` archive
    #[arg(short, long)]
    output: Option<String>,

    /// Explicitly define the project type (e.g., 'mcp' for legacy host servers)
    #[arg(short = 't', long = "type")]
    project_type: Option<String>,

    /// Import a legacy `mcp.json` to auto-convert
    #[arg(long)]
    from_mcp_json: Option<String>,

    /// Internal: run Wizer on the embedded QuickJS kernel (used by compiler subprocess)
    #[arg(long, hide = true)]
    wizer_internal: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Handle wizer-internal (hidden, used by compiler subprocess)
    if let Some(output) = args.wizer_internal {
        astrid_openclaw::compiler::run_wizer_internal(&output)
            .map_err(|e| anyhow::anyhow!("wizer-internal failed: {e}"))?;
        return Ok(());
    }

    build::run_build(
        args.path.as_deref(),
        args.output.as_deref(),
        args.project_type.as_deref(),
        args.from_mcp_json.as_deref(),
    )
}
