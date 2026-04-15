//! `OpenClaw` JS/TS capsule builder — compiles via the `OpenClaw` pipeline.

use crate::archiver::pack_capsule_archive;
use anyhow::{Context, Result};
use astrid_openclaw::pipeline::{self, CompileOptions};
use astrid_openclaw::tier::PluginTier;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Build an `OpenClaw` JS/TS plugin into a `.capsule` archive.
///
/// Delegates to `astrid_openclaw::pipeline::compile_plugin` for the heavy lifting,
/// then packages the output directory into a `.capsule` tar.gz via the archiver.
pub(crate) fn build(dir: &Path, output: Option<&str>) -> Result<()> {
    info!("Building OpenClaw JS/TS capsule from {}", dir.display());

    let build_dir = tempfile::tempdir().context("Failed to create temp build directory")?;
    let config = HashMap::<String, serde_json::Value>::new();
    let cache_dir = pipeline::default_cache_dir();

    let opts = CompileOptions {
        plugin_dir: dir,
        output_dir: build_dir.path(),
        config: &config,
        cache_dir: cache_dir.as_deref(),
        js_only: false,
        no_cache: false,
    };

    let result = pipeline::compile_plugin(&opts)
        .map_err(|e| anyhow::anyhow!("OpenClaw compilation failed: {e}"))?;

    let tier_label = match result.tier {
        PluginTier::Wasm => "Tier 1 (WASM)",
        PluginTier::Node => "Tier 2 (Node.js MCP)",
    };
    info!(
        "   Compiled {} as {} (cached: {})",
        result.astrid_id, tier_label, result.cached
    );

    // Read the generated Capsule.toml
    let capsule_toml_path = build_dir.path().join("Capsule.toml");
    let toml_content = fs::read_to_string(&capsule_toml_path)
        .context("Compilation succeeded but no Capsule.toml was generated")?;

    // Determine the output location
    let out_dir = match output {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?.join("dist"),
    };
    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)?;
    }
    let out_file = out_dir.join(format!("{}.capsule", result.astrid_id));

    // Collect artifacts based on tier
    match result.tier {
        PluginTier::Wasm => {
            let wasm_path = build_dir.path().join("plugin.wasm");
            pack_capsule_archive(&out_file, &toml_content, Some(&wasm_path), dir, &[], None)?;
        },
        PluginTier::Node => {
            // Tier 2: include the entire build output (source tree, node_modules,
            // package.json, bridge script, etc.) — everything except Capsule.toml
            // which is written separately by the archiver.
            let mut additional: Vec<PathBuf> = Vec::new();
            for entry in
                fs::read_dir(build_dir.path()).context("Failed to read Tier 2 build directory")?
            {
                let entry = entry?;
                let name = entry.file_name();
                if name == "Capsule.toml" {
                    continue; // written separately by pack_capsule_archive
                }
                additional.push(entry.path());
            }
            let refs: Vec<&Path> = additional.iter().map(PathBuf::as_path).collect();
            pack_capsule_archive(
                &out_file,
                &toml_content,
                None,
                build_dir.path(),
                &refs,
                None,
            )?;
        },
    }

    info!(
        "Successfully built OpenClaw capsule: {}",
        out_file.display()
    );
    Ok(())
}
