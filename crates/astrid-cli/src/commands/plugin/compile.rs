//! Plugin management commands - install, remove, list, compile, and inspect plugins.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use astrid_core::dirs::AstridHome;
use astrid_plugins::lockfile::LockedPlugin;
use openclaw_bridge::tier::{PluginTier, detect_tier};

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use super::install::compile_openclaw;

pub(crate) fn compile_plugin(path: &str, output: Option<&str>) -> anyhow::Result<()> {
    let source_path = Path::new(path);
    if !source_path.exists() {
        bail!("Source path does not exist: {path}");
    }

    let home = AstridHome::resolve()?;

    // Detect source type
    if source_path.is_dir() && source_path.join("openclaw.plugin.json").exists() {
        // OpenClaw plugin directory
        let out_dir = output.map_or_else(|| source_path.join("dist"), PathBuf::from);

        let oc_manifest = openclaw_bridge::manifest::parse_manifest(source_path)
            .context("failed to parse openclaw.plugin.json")?;

        // Check if this plugin requires Tier 2 (Node.js)
        let tier = detect_tier(source_path, Some(&oc_manifest));
        if tier == PluginTier::Node {
            bail!(
                "Plugin detected as Tier 2 (Node.js). Tier 2 plugins cannot be compiled to WASM.\n\
                 Use `astrid plugin install` to install via the Node.js bridge instead."
            );
        }

        println!(
            "{}",
            Theme::info(&format!("Compiling OpenClaw plugin at: {path}"))
        );
        let astrid_id = compile_openclaw(source_path, &out_dir, &home, &oc_manifest)?;
        let wasm_path = out_dir.join("plugin.wasm");
        let meta = std::fs::metadata(&wasm_path)?;
        let hash = LockedPlugin::compute_wasm_hash(&wasm_path)?;

        println!("{}", Theme::success("Compilation complete"));
        println!("{}", Theme::kv("Plugin ID", &astrid_id));
        println!("{}", Theme::kv("Output", &out_dir.display().to_string()));
        println!("{}", Theme::kv("WASM Hash", &hash));
        #[allow(clippy::cast_precision_loss)]
        let size_kb = meta.len() as f64 / 1024.0;
        println!("{}", Theme::kv("WASM Size", &format!("{size_kb:.1} KB")));
    } else if source_path.is_file() {
        // Bare JS/TS file
        let ext = source_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "js" | "ts" | "jsx" | "tsx") {
            bail!("Unsupported file type: .{ext} (expected .js, .ts, .jsx, or .tsx)");
        }

        let out_dir = output.map_or_else(
            || source_path.parent().unwrap_or(Path::new(".")).join("dist"),
            PathBuf::from,
        );

        println!("{}", Theme::info(&format!("Compiling {ext} file: {path}")));

        let raw_source = std::fs::read_to_string(source_path)
            .with_context(|| format!("failed to read {path}"))?;

        let filename = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plugin.js");

        let js = openclaw_bridge::transpiler::transpile(&raw_source, filename)
            .context("transpilation failed")?;

        let config: HashMap<String, serde_json::Value> = HashMap::new();
        let shimmed = openclaw_bridge::shim::generate(&js, &config);

        std::fs::create_dir_all(&out_dir)?;
        let wasm_path = out_dir.join("plugin.wasm");
        openclaw_bridge::compiler::compile(&shimmed, &wasm_path)
            .context("WASM compilation failed")?;

        let meta = std::fs::metadata(&wasm_path)?;
        let hash = LockedPlugin::compute_wasm_hash(&wasm_path)?;

        println!("{}", Theme::success("Compilation complete"));
        println!("{}", Theme::kv("Output", &wasm_path.display().to_string()));
        println!("{}", Theme::kv("WASM Hash", &hash));
        #[allow(clippy::cast_precision_loss)]
        let size_kb = meta.len() as f64 / 1024.0;
        println!("{}", Theme::kv("WASM Size", &format!("{size_kb:.1} KB")));
    } else {
        bail!(
            "Cannot detect plugin type at '{path}'. Expected:\n\
             - Directory with openclaw.plugin.json (OpenClaw plugin)\n\
             - .js/.ts/.jsx/.tsx file (bare script)"
        );
    }

    Ok(())
}
