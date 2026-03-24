//! Bundled build binary — installed alongside `astrid` via `cargo install astrid`.
//!
//! Delegates to the shared `astrid_build` library. This is identical to the
//! standalone `astrid-build` binary but co-installed with the CLI so
//! `find_companion_binary("astrid-build")` always finds it.

use clap::Parser;

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

    /// Internal: run Wizer on the embedded `QuickJS` kernel (used by compiler subprocess)
    #[arg(long, hide = true)]
    wizer_internal: Option<std::path::PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if let Some(output) = args.wizer_internal {
        return astrid_build::run_wizer_internal(&output);
    }

    astrid_build::run(
        args.path.as_deref(),
        args.output.as_deref(),
        args.project_type.as_deref(),
        args.from_mcp_json.as_deref(),
    )
}
