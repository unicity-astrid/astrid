//! CLI entry point for the `OpenClaw` → Astralis bridge.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};

use openclaw_bridge::bundler;
use openclaw_bridge::compiler;
use openclaw_bridge::error::{BridgeError, BridgeResult};
use openclaw_bridge::manifest;
use openclaw_bridge::output;
use openclaw_bridge::shim;

#[derive(Parser)]
#[command(
    name = "openclaw-bridge",
    about = "Convert OpenClaw tool plugins into Astralis WASM plugins"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert an `OpenClaw` plugin to an Astralis WASM plugin.
    Convert {
        /// Path to the `OpenClaw` plugin directory (containing `openclaw.plugin.json`).
        #[arg(long)]
        plugin_dir: PathBuf,

        /// Output directory for the generated Astralis plugin. Defaults to `./output`.
        #[arg(long, default_value = "output")]
        output: PathBuf,

        /// Plugin configuration as a JSON object (e.g. '{"apiKey":"...","timeout":30}').
        #[arg(long)]
        config: Option<String>,

        /// Only generate the JS shim (skip WASM compilation).
        #[arg(long)]
        js_only: bool,

        /// Skip esbuild bundling (use the entry point JS file directly).
        #[arg(long)]
        skip_bundle: bool,
    },

    /// Check that required external tools are installed.
    Doctor,
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
            skip_bundle,
        } => run_convert(
            &plugin_dir,
            &output_dir,
            config.as_deref(),
            js_only,
            skip_bundle,
        ),
        Command::Doctor => {
            run_doctor();
            Ok(())
        },
    }
}

fn run_convert(
    plugin_dir: &Path,
    output_dir: &Path,
    config_json: Option<&str>,
    js_only: bool,
    skip_bundle: bool,
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
        oc_manifest.name, oc_manifest.version
    );

    // 3. Resolve the entry point file
    let entry_point = plugin_dir.join(&oc_manifest.main);
    if !entry_point.exists() {
        return Err(BridgeError::EntryPointNotFound(entry_point));
    }

    // 4. Bundle if needed
    let js_code = if skip_bundle {
        std::fs::read_to_string(&entry_point)?
    } else {
        // Always run through esbuild for a self-contained CJS bundle
        bundler::bundle(&entry_point)?
    };

    // 5. Generate JS shim
    let astralis_id = manifest::convert_id(&oc_manifest.id)?;
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
        // 7. Compile to WASM
        let wasm_path = output_dir.join("plugin.wasm");
        compiler::compile(&shim_path, &wasm_path)?;
        eprintln!("Compiled WASM: {}", wasm_path.display());

        // 8. Generate plugin.toml
        output::generate_manifest(&astralis_id, &oc_manifest, &wasm_path, &config, output_dir)?;
        eprintln!("Wrote plugin.toml");
    }

    eprintln!("Done.");
    Ok(())
}

fn run_doctor() {
    let mut all_ok = true;

    // Check extism-js
    if let Ok(path) = which::which("extism-js") {
        eprintln!("[ok] extism-js found: {}", path.display());
    } else {
        eprintln!("[missing] extism-js — Extism JS PDK compiler");
        eprintln!("  Install: https://github.com/nicholasgasior/extism-js/releases");
        all_ok = false;
    }

    // Check esbuild
    if let Ok(path) = which::which("esbuild") {
        eprintln!("[ok] esbuild found: {}", path.display());
    } else {
        eprintln!("[missing] esbuild — JS/TS bundler");
        eprintln!("  Install: npm i -g esbuild");
        all_ok = false;
    }

    if all_ok {
        eprintln!("\nAll tools installed.");
    } else {
        eprintln!("\nSome tools are missing. Install them before running `convert`.");
    }
}
