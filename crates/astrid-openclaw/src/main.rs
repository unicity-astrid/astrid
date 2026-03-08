//! CLI entry point for the `OpenClaw` → Astrid bridge.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

use astrid_openclaw::compiler;
use astrid_openclaw::error::BridgeResult;
use astrid_openclaw::pipeline::{self, CompileOptions};

#[derive(Parser)]
#[command(
    name = "astrid-openclaw",
    about = "Convert OpenClaw tool plugins into Astrid WASM plugins"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert an `OpenClaw` plugin to an Astrid WASM plugin.
    Convert {
        /// Path to the `OpenClaw` plugin directory (containing `openclaw.plugin.json`).
        #[arg(long)]
        plugin_dir: PathBuf,

        /// Output directory for the generated Astrid plugin. Defaults to `./output`.
        #[arg(long, default_value = "output")]
        output: PathBuf,

        /// Plugin configuration as a JSON object (e.g. '{"apiKey":"...","timeout":30}').
        #[arg(long)]
        config: Option<String>,

        /// Only generate the JS shim (skip WASM compilation).
        #[arg(long)]
        js_only: bool,

        /// Disable compilation cache.
        #[arg(long)]
        no_cache: bool,
    },

    /// Run garbage collection on the compilation cache.
    CacheGc {
        /// Maximum age of cache entries in days.
        #[arg(long, default_value = "30")]
        max_age_days: u64,

        /// Maximum total cache size in bytes.
        #[arg(long, default_value = "500000000")]
        max_size_bytes: u64,
    },

    /// Internal: run Wizer on the embedded `QuickJS` kernel (hidden, used by compiler subprocess).
    #[command(hide = true)]
    WizerInternal {
        /// Output path for the Wizer'd WASM.
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run(cli: Cli) -> BridgeResult<()> {
    match cli.command {
        Command::Convert {
            plugin_dir,
            output,
            config,
            js_only,
            no_cache,
        } => {
            let config: HashMap<String, serde_json::Value> = match config.as_deref() {
                Some(json) => serde_json::from_str(json).map_err(|e| {
                    astrid_openclaw::error::BridgeError::Manifest(format!(
                        "invalid --config JSON: {e}"
                    ))
                })?,
                None => HashMap::new(),
            };

            let cache_dir = if no_cache {
                None
            } else {
                pipeline::default_cache_dir()
            };

            let opts = CompileOptions {
                plugin_dir: &plugin_dir,
                output_dir: &output,
                config: &config,
                cache_dir: cache_dir.as_deref(),
                js_only,
                no_cache,
            };

            let result = pipeline::compile_plugin(&opts)?;

            eprintln!(
                "Compiled: {} v{} (tier: {}, cached: {})",
                result.manifest.display_name(),
                result.manifest.display_version(),
                result.tier,
                result.cached,
            );
            eprintln!("Output: {}", output.display());
            Ok(())
        },
        Command::CacheGc {
            max_age_days,
            max_size_bytes,
        } => {
            let cache_dir = pipeline::default_cache_dir().ok_or_else(|| {
                astrid_openclaw::error::BridgeError::Cache(
                    "could not determine home directory for cache".into(),
                )
            })?;

            let stats = pipeline::cache_gc(&cache_dir, max_age_days, max_size_bytes)?;
            eprintln!(
                "Cache GC: removed {} entries, freed {} bytes",
                stats.entries_removed, stats.bytes_freed
            );
            Ok(())
        },
        Command::WizerInternal { output } => compiler::run_wizer_internal(&output),
    }
}
