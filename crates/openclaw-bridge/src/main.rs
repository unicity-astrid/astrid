//! CLI entry point for the `OpenClaw` → Astrid bridge.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};

use openclaw_bridge::compiler;
use openclaw_bridge::error::{BridgeError, BridgeResult};
use openclaw_bridge::manifest;
use openclaw_bridge::output;
use openclaw_bridge::shim;
use openclaw_bridge::transpiler;

#[derive(Parser)]
#[command(
    name = "openclaw-bridge",
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
            output: output_dir,
            config,
            js_only,
        } => run_convert(&plugin_dir, &output_dir, config.as_deref(), js_only),
        Command::WizerInternal { output } => compiler::run_wizer_internal(&output),
    }
}

fn run_convert(
    plugin_dir: &Path,
    output_dir: &Path,
    config_json: Option<&str>,
    js_only: bool,
) -> BridgeResult<()> {
    // 1. Parse config from --config flag
    let config: HashMap<String, serde_json::Value> = match config_json {
        Some(json) => serde_json::from_str(json)
            .map_err(|e| BridgeError::Manifest(format!("invalid --config JSON: {e}")))?,
        None => HashMap::new(),
    };

    // 2. Parse OpenClaw manifest
    let oc_manifest = manifest::parse_manifest(plugin_dir)?;
    eprintln!(
        "Parsed manifest: {} v{}",
        oc_manifest.display_name(),
        oc_manifest.display_version()
    );

    // 3. Resolve the entry point file
    let entry_point_rel = manifest::resolve_entry_point(plugin_dir)?;
    let entry_point = plugin_dir.join(&entry_point_rel);
    if !entry_point.exists() {
        return Err(BridgeError::EntryPointNotFound(entry_point));
    }

    // 4. Read and transpile source (TS→JS + import rejection)
    let raw_source = std::fs::read_to_string(&entry_point)?;
    let js_code = transpiler::transpile(&raw_source, &entry_point_rel)?;
    eprintln!("Transpiled: {entry_point_rel}");

    // 5. Generate JS shim
    let astrid_id = manifest::convert_id(&oc_manifest.id)?;
    let shim_code = shim::generate(&js_code, &config);

    // 6. Write output directory
    std::fs::create_dir_all(output_dir)?;

    // Always write the shim for debugging
    let shim_path = output_dir.join("shim.js");
    std::fs::write(&shim_path, &shim_code)?;
    eprintln!("Wrote shim: {}", shim_path.display());

    if js_only {
        eprintln!("--js-only: skipping WASM compilation");
    } else {
        // 7. Compile to WASM (embedded Wizer + kernel)
        let wasm_path = output_dir.join("plugin.wasm");
        compiler::compile(&shim_code, &wasm_path)?;
        eprintln!("Compiled WASM: {}", wasm_path.display());

        // 8. Generate plugin.toml
        output::generate_manifest(&astrid_id, &oc_manifest, &wasm_path, &config, output_dir)?;
        eprintln!("Wrote plugin.toml");
    }

    eprintln!("Done.");
    Ok(())
}
