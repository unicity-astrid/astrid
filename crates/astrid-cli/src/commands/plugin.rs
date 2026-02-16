//! Plugin management commands - install, remove, list, compile, and inspect plugins.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use astrid_core::dirs::AstridHome;
use astrid_plugins::git_install::GitSource;
use astrid_plugins::lockfile::{LOCKFILE_NAME, LockedPlugin, PluginLockfile, PluginSource};
use astrid_plugins::manifest::{PluginCapability, PluginEntryPoint};
use astrid_plugins::npm::{NpmFetcher, NpmSpec};
use astrid_plugins::plugin::PluginId;
use astrid_plugins::{discover_manifests, load_manifest};
use openclaw_bridge::tier::{PluginTier, detect_tier};

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine the target installation directory for a plugin.
fn resolve_target_dir(home: &AstridHome, id: &str, workspace: bool) -> anyhow::Result<PathBuf> {
    if workspace {
        let cwd = std::env::current_dir()
            .context("failed to get current directory")?
            .canonicalize()
            .context("failed to canonicalize current directory")?;
        Ok(cwd.join(".astrid/plugins").join(id))
    } else {
        Ok(home.plugins_dir().join(id))
    }
}

/// Determine the lockfile path (user-level or workspace-level).
fn resolve_lockfile_path(home: &AstridHome, workspace: bool) -> anyhow::Result<PathBuf> {
    if workspace {
        let cwd = std::env::current_dir()
            .context("failed to get current directory")?
            .canonicalize()
            .context("failed to canonicalize current directory")?;
        Ok(cwd.join(".astrid").join(LOCKFILE_NAME))
    } else {
        Ok(home.root().join(LOCKFILE_NAME))
    }
}

/// Find an installed plugin directory by checking user-level then workspace-level.
fn find_plugin_dir(home: &AstridHome, id: &str) -> anyhow::Result<PathBuf> {
    // User-level
    let user_dir = home.plugins_dir().join(id);
    if user_dir.join("plugin.toml").exists() {
        return Ok(user_dir);
    }

    // Workspace-level (canonicalize to ensure absolute path)
    let cwd = std::env::current_dir()
        .context("failed to get current directory")?
        .canonicalize()
        .context("failed to canonicalize current directory")?;
    let ws_dir = cwd.join(".astrid/plugins").join(id);
    if ws_dir.join("plugin.toml").exists() {
        return Ok(ws_dir);
    }

    bail!(
        "Plugin '{id}' not found. Checked:\n  {}\n  {}",
        user_dir.display(),
        ws_dir.display()
    )
}

/// Recursively copy a directory tree.
///
/// Rejects symlinks — plugin sources must not contain symlinks for security.
fn copy_plugin_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_symlink() {
            bail!(
                "plugin source contains a symlink at {}, which is not allowed",
                src_path.display()
            );
        }

        if file_type.is_dir() {
            // Skip directories that would be recreated or are not needed
            let name = entry.file_name();
            if name == "node_modules" || name == ".git" || name == "dist" {
                continue;
            }
            copy_plugin_dir(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .with_context(|| format!("failed to copy {}", src_path.display()))?;
        }
    }
    Ok(())
}

/// Atomically swap a staging directory into the target, then update the lockfile.
///
/// Order: backup existing target → rename staging → update lockfile.
/// On any failure the rename is rolled back and the backup restored.
fn atomic_install(
    staging: tempfile::TempDir,
    target_dir: &Path,
    locked: &LockedPlugin,
    lockfile_path: &Path,
) -> anyhow::Result<()> {
    // Backup existing target if present.
    let backup = if target_dir.exists() {
        let mut backup_name = target_dir
            .file_name()
            .context("target dir has no file name")?
            .to_os_string();
        backup_name.push("-backup");
        let backup_path = target_dir.with_file_name(backup_name);
        // Clean up any stale backup from a previous failed install.
        if backup_path.exists() {
            std::fs::remove_dir_all(&backup_path).with_context(|| {
                format!(
                    "failed to remove stale backup directory {}",
                    backup_path.display()
                )
            })?;
        }
        std::fs::rename(target_dir, &backup_path)
            .with_context(|| format!("failed to backup {}", target_dir.display()))?;
        Some(backup_path)
    } else {
        None
    };

    let staging_path = staging.keep();
    if let Err(e) = std::fs::rename(&staging_path, target_dir) {
        // Rollback: restore backup if we made one.
        if let Some(ref bp) = backup {
            let _ = std::fs::rename(bp, target_dir);
        }
        let _ = std::fs::remove_dir_all(&staging_path);
        return Err(e)
            .with_context(|| format!("failed to rename staging dir to {}", target_dir.display()));
    }

    // Rename succeeded — now update lockfile under a single exclusive lock
    // to prevent TOCTOU races between concurrent install/remove operations.
    if let Err(e) = PluginLockfile::update(lockfile_path, |lockfile| {
        lockfile.add(locked.clone());
        Ok(())
    }) {
        // Rollback: undo the rename, restore backup.
        let _ = std::fs::rename(target_dir, &staging_path);
        if let Some(ref bp) = backup {
            let _ = std::fs::rename(bp, target_dir);
        }
        let _ = std::fs::remove_dir_all(&staging_path);
        return Err(e).context("failed to save lockfile after install");
    }

    // Success — remove backup.
    if let Some(ref bp) = backup {
        let _ = std::fs::remove_dir_all(bp);
    }

    Ok(())
}

/// Run the `OpenClaw` compilation pipeline on a source directory.
///
/// Returns the Astrid plugin ID derived from the `OpenClaw` manifest.
fn compile_openclaw(
    source_dir: &Path,
    output_dir: &Path,
    home: &AstridHome,
    oc_manifest: &openclaw_bridge::manifest::OpenClawManifest,
) -> anyhow::Result<String> {
    let astrid_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
        .context("failed to convert OpenClaw ID to Astrid ID")?;

    let entry_point = openclaw_bridge::manifest::resolve_entry_point(source_dir)
        .context("failed to resolve plugin entry point")?;
    let main_path = source_dir.join(&entry_point);
    let raw_source = std::fs::read_to_string(&main_path)
        .with_context(|| format!("failed to read entry point: {}", main_path.display()))?;

    // Check compilation cache — hash both the entry point source and the
    // OpenClaw manifest so that manifest changes (id, version, capabilities)
    // invalidate the cache even when the JS source is unchanged.
    // NOTE: this only covers the main entry point, not transitive imports.
    // Plugins that import other local files may get stale cache hits if only
    // the imported file changed. A full-source-tree hash is a future improvement.
    let kernel_hash = openclaw_bridge::compiler::kernel_hash();
    let cache = openclaw_bridge::cache::CompilationCache::new(home.plugin_cache_dir(), kernel_hash);
    let manifest_path = source_dir.join("openclaw.plugin.json");
    let manifest_contents = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(raw_source.as_bytes());
    hasher.update(manifest_contents.as_bytes());
    let source_hash = hasher.finalize().to_hex().to_string();

    if let Some(hit) = cache.lookup(&source_hash, openclaw_bridge::VERSION) {
        println!("{}", Theme::dimmed("  Cache hit — skipping compilation"));
        std::fs::create_dir_all(output_dir)
            .with_context(|| format!("failed to create {}", output_dir.display()))?;
        let wasm_path = output_dir.join("plugin.wasm");
        std::fs::write(&wasm_path, &hit.wasm).context("failed to write cached WASM")?;
        std::fs::write(output_dir.join("plugin.toml"), &hit.manifest)
            .context("failed to write cached manifest")?;
        return Ok(astrid_id);
    }

    // Full pipeline: transpile → shim → compile → generate_manifest
    let js = openclaw_bridge::transpiler::transpile(&raw_source, &entry_point)
        .context("transpilation failed")?;

    let config: HashMap<String, serde_json::Value> = HashMap::new();
    let shimmed = openclaw_bridge::shim::generate(&js, &config);

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let wasm_path = output_dir.join("plugin.wasm");
    openclaw_bridge::compiler::compile(&shimmed, &wasm_path).context("WASM compilation failed")?;

    openclaw_bridge::output::generate_manifest(
        &astrid_id,
        oc_manifest,
        &wasm_path,
        &config,
        output_dir,
    )
    .context("manifest generation failed")?;

    // Store in cache for next time
    let wasm_bytes = std::fs::read(&wasm_path)?;
    let manifest_str = std::fs::read_to_string(output_dir.join("plugin.toml"))?;
    if let Err(e) = cache.store(
        &source_hash,
        openclaw_bridge::VERSION,
        &wasm_bytes,
        &manifest_str,
    ) {
        eprintln!(
            "{}",
            Theme::warning(&format!("Cache store failed (non-fatal): {e}"))
        );
    }

    Ok(astrid_id)
}

/// Prepare an `OpenClaw` plugin for Tier 2 (Node.js MCP bridge) installation.
///
/// Steps:
/// 1. Copy source to output directory
/// 2. Pre-transpile all `.ts`/`.tsx` files to `.js` using OXC
/// 3. Write the universal MCP bridge script
/// 4. Run `npm install --omit=dev --ignore-scripts` if `package.json` exists
/// 5. Generate `plugin.toml` with MCP entry point
///
/// Returns the Astrid plugin ID.
fn prepare_tier2(
    source_dir: &Path,
    output_dir: &Path,
    _home: &AstridHome,
    oc_manifest: &openclaw_bridge::manifest::OpenClawManifest,
) -> anyhow::Result<String> {
    let astrid_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
        .context("failed to convert OpenClaw ID")?;

    let entry_point = openclaw_bridge::manifest::resolve_entry_point(source_dir)
        .context("failed to resolve plugin entry point")?;

    // Copy source to output dir (we'll modify files in-place for transpilation)
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    copy_plugin_dir(source_dir, output_dir)?;

    // Pre-transpile TS→JS in-place using OXC
    // Transpile TS→JS in the entire output dir (not just src/ — entry points may be at root)
    transpile_ts_in_dir(output_dir)?;

    // Rewrite main entry point extension from .ts/.tsx to .js
    let main_path = Path::new(&entry_point);
    let is_ts = main_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ts") || ext.eq_ignore_ascii_case("tsx"));
    let main_entry = if is_ts {
        main_path.with_extension("js").to_string_lossy().to_string()
    } else {
        entry_point
    };

    // Write the universal bridge script
    openclaw_bridge::node_bridge::write_bridge_script(output_dir)
        .context("failed to write bridge script")?;

    // Run npm install if package.json exists
    if output_dir.join("package.json").exists() {
        println!(
            "{}",
            Theme::dimmed("  Running npm install --omit=dev --ignore-scripts...")
        );
        let npm_output = std::process::Command::new("npm")
            .args(["install", "--omit=dev", "--ignore-scripts"])
            .current_dir(output_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .context("failed to run npm install (is npm installed?)")?;

        if !npm_output.status.success() {
            let stderr = String::from_utf8_lossy(&npm_output.stderr);
            bail!("npm install failed (exit {}):\n{stderr}", npm_output.status);
        }
    }

    // Generate plugin.toml with MCP entry point
    let manifest = astrid_plugins::manifest::PluginManifest {
        id: PluginId::new(&astrid_id).context("invalid plugin ID")?,
        name: oc_manifest.display_name().to_string(),
        version: oc_manifest.display_version().to_string(),
        description: oc_manifest.description.clone(),
        author: None,
        entry_point: PluginEntryPoint::Mcp {
            command: "node".into(),
            args: vec![
                "astrid_bridge.mjs".into(),
                "--entry".into(),
                format!("./{}", main_entry.strip_prefix("./").unwrap_or(&main_entry)),
                "--plugin-id".into(),
                astrid_id.clone(),
            ],
            env: HashMap::new(),
            binary_hash: None,
        },
        capabilities: vec![],
        config: HashMap::new(),
    };

    let manifest_toml =
        toml::to_string_pretty(&manifest).context("failed to serialize plugin manifest")?;
    std::fs::write(output_dir.join("plugin.toml"), manifest_toml)
        .context("failed to write plugin.toml")?;

    Ok(astrid_id)
}

/// Recursively transpile all `.ts` and `.tsx` files in a directory to `.js` using OXC.
///
/// The original `.ts` file is removed after successful transpilation.
fn transpile_ts_in_dir(dir: &Path) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in
        std::fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let name = entry.file_name();
            if name == "node_modules" || name == "dist" || name == ".git" {
                continue;
            }
            transpile_ts_in_dir(&path)?;
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "ts" && ext != "tsx" {
            continue;
        }

        // Skip TypeScript declaration files (.d.ts / .d.tsx)
        if path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|stem| Path::new(stem).extension())
            .is_some_and(|e| e.eq_ignore_ascii_case("d"))
        {
            continue;
        }

        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.ts");

        let js = transpile_lenient(&source, filename)
            .with_context(|| format!("failed to transpile {}", path.display()))?;

        let js_path = path.with_extension("js");
        std::fs::write(&js_path, js)
            .with_context(|| format!("failed to write {}", js_path.display()))?;

        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }

    Ok(())
}

/// Transpile `TypeScript` to `JavaScript`, allowing import statements.
///
/// Unlike `openclaw_bridge::transpiler::transpile()`, this does NOT reject
/// runtime imports — Tier 2 plugins have npm dependencies available at runtime.
fn transpile_lenient(source: &str, filename: &str) -> anyhow::Result<String> {
    use oxc::codegen::Codegen;
    use oxc::parser::Parser;
    use oxc::semantic::SemanticBuilder;
    use oxc::span::SourceType;
    use oxc::transformer::{TransformOptions, Transformer};

    let allocator = oxc_allocator::Allocator::default();
    let source_type = SourceType::from_path(filename).unwrap_or_else(|_| SourceType::mjs());

    let parse_ret = Parser::new(&allocator, source, source_type).parse();
    if parse_ret.panicked || !parse_ret.errors.is_empty() {
        let errors: Vec<String> = parse_ret.errors.iter().map(|e| format!("{e}")).collect();
        bail!("parse errors:\n{}", errors.join("\n"));
    }

    let mut program = parse_ret.program;

    let sem_ret = SemanticBuilder::new()
        .with_excess_capacity(2.0)
        .build(&program);
    let scoping = sem_ret.semantic.into_scoping();

    let transform_options = TransformOptions::default();
    let path = std::path::Path::new(filename);
    let transform_ret = Transformer::new(&allocator, path, &transform_options)
        .build_with_scoping(scoping, &mut program);

    if !transform_ret.errors.is_empty() {
        let errors: Vec<String> = transform_ret
            .errors
            .iter()
            .map(|e| format!("{e}"))
            .collect();
        bail!("transform errors:\n{}", errors.join("\n"));
    }

    let codegen_ret = Codegen::new().build(&program);
    Ok(codegen_ret.code)
}

/// Best-effort daemon notification. Prints a warning on failure, never fails the command.
async fn notify_daemon(action: &str, plugin_id: &str) {
    match crate::daemon_client::DaemonClient::connect().await {
        Ok(client) => {
            let result = match action {
                "load" => client.load_plugin(plugin_id).await.map(|_| ()),
                "unload" => client.unload_plugin(plugin_id).await,
                _ => Ok(()),
            };
            if let Err(e) = result {
                eprintln!(
                    "{}",
                    Theme::warning(&format!("Daemon {action} notification failed: {e}"))
                );
            }
        },
        Err(_) => {
            println!(
                "{}",
                Theme::dimmed("  Daemon not running — plugin will be loaded on next start")
            );
        },
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Show detailed information about an installed plugin.
pub(crate) fn plugin_info(id: &str) -> anyhow::Result<()> {
    let plugin_id = PluginId::new(id).context("invalid plugin ID")?;
    let id = plugin_id.as_str();
    let home = AstridHome::resolve()?;

    let plugin_dir = find_plugin_dir(&home, id)?;
    let manifest = load_manifest(&plugin_dir.join("plugin.toml"))
        .with_context(|| format!("failed to load manifest for '{id}'"))?;

    println!("{}", Theme::header(&format!("Plugin: {id}")));
    println!("{}", Theme::kv("Name", &manifest.name));
    println!("{}", Theme::kv("Version", &manifest.version));

    if let Some(desc) = &manifest.description {
        println!("{}", Theme::kv("Description", desc));
    }
    if let Some(author) = &manifest.author {
        println!("{}", Theme::kv("Author", author));
    }

    // Entry point info
    match &manifest.entry_point {
        PluginEntryPoint::Wasm { path, hash } => {
            println!("{}", Theme::kv("Type", "WASM"));
            println!("{}", Theme::kv("Entry", &path.display().to_string()));
            if let Some(h) = hash {
                println!("{}", Theme::kv("Manifest Hash", h));
            }
            // Compute live hash
            let wasm_path = if path.is_absolute() {
                path.clone()
            } else {
                plugin_dir.join(path)
            };
            if wasm_path.exists() {
                match LockedPlugin::compute_wasm_hash(&wasm_path) {
                    Ok(live_hash) => println!("{}", Theme::kv("WASM Hash", &live_hash)),
                    Err(e) => eprintln!("{}", Theme::warning(&format!("Could not hash WASM: {e}"))),
                }
                if let Ok(meta) = std::fs::metadata(&wasm_path) {
                    #[allow(clippy::cast_precision_loss)]
                    let size_kb = meta.len() as f64 / 1024.0;
                    println!("{}", Theme::kv("WASM Size", &format!("{size_kb:.1} KB")));
                }
            }
        },
        PluginEntryPoint::Mcp { command, args, .. } => {
            println!("{}", Theme::kv("Type", "MCP (native)"));
            let cmd_str = if args.is_empty() {
                command.clone()
            } else {
                format!("{command} {}", args.join(" "))
            };
            println!("{}", Theme::kv("Command", &cmd_str));
        },
    }

    // Capabilities
    if !manifest.capabilities.is_empty() {
        println!("\n{}", Theme::kv("Capabilities", ""));
        for cap in &manifest.capabilities {
            let desc = match cap {
                PluginCapability::HttpAccess { hosts } => {
                    format!("http_access ({})", hosts.join(", "))
                },
                PluginCapability::FileRead { paths } => format!("file_read ({})", paths.join(", ")),
                PluginCapability::FileWrite { paths } => {
                    format!("file_write ({})", paths.join(", "))
                },
                PluginCapability::KvStore => "kv_store".to_string(),
                PluginCapability::Config => "config".to_string(),
            };
            println!("  - {desc}");
        }
    }

    // Lockfile source info
    let cwd = std::env::current_dir()
        .context("failed to get current directory")?
        .canonicalize()
        .context("failed to canonicalize current directory")?;
    let lockfile_paths = [
        home.root().join(LOCKFILE_NAME),
        cwd.join(".astrid").join(LOCKFILE_NAME),
    ];
    for lf_path in &lockfile_paths {
        if let Ok(lf) = PluginLockfile::load(lf_path)
            && let Some(entry) = lf.get(&plugin_id)
        {
            println!("\n{}", Theme::kv("Source", &entry.source.to_string()));
            println!(
                "{}",
                Theme::kv(
                    "Installed",
                    &entry.installed_at.format("%Y-%m-%d %H:%M UTC").to_string()
                )
            );
            println!("{}", Theme::kv("Lockfile Hash", &entry.wasm_hash));
            break;
        }
    }

    println!(
        "{}",
        Theme::kv("Location", &plugin_dir.display().to_string())
    );

    Ok(())
}

/// List installed plugins with state information.
///
/// Connects to the daemon via JSON-RPC if running (live state),
/// falls back to manifest scan (static) otherwise.
pub(crate) async fn list_plugins() -> anyhow::Result<()> {
    // Try daemon first for live state
    if let Ok(client) = crate::daemon_client::DaemonClient::connect().await {
        match client.list_plugins().await {
            Ok(plugins) => {
                if plugins.is_empty() {
                    println!("{}", Theme::info("No plugins installed"));
                    return Ok(());
                }
                println!("{}", Theme::header("Installed Plugins (live)"));
                println!(
                    "  {:<20} {:<10} {:<10} {:>5}",
                    "ID", "VERSION", "STATE", "TOOLS"
                );
                println!("{}", Theme::separator());
                for p in &plugins {
                    let state_display = match p.state.as_str() {
                        "ready" => Theme::success(&p.state),
                        "failed" => Theme::error(&p.state),
                        "loading" => Theme::warning(&p.state),
                        _ => Theme::dimmed(&p.state),
                    };
                    println!(
                        "  {:<20} {:<10} {:<10} {:>5}",
                        p.id, p.version, state_display, p.tool_count
                    );
                }
                println!(
                    "\n{}",
                    Theme::dimmed(&format!("{} plugin(s)", plugins.len()))
                );
                return Ok(());
            },
            Err(e) => {
                eprintln!(
                    "{}",
                    Theme::dimmed(&format!(
                        "Daemon query failed: {e} — falling back to manifest scan"
                    ))
                );
            },
        }
    }

    // Fallback: static manifest scan
    let home = AstridHome::resolve()?;
    let extra = vec![home.plugins_dir()];
    let discovered = discover_manifests(Some(&extra));

    if discovered.is_empty() {
        println!("{}", Theme::info("No plugins installed"));
        return Ok(());
    }

    // Load lockfile for source annotations (best-effort — don't create artifacts)
    let lockfile =
        PluginLockfile::load_or_default(&home.root().join(LOCKFILE_NAME)).unwrap_or_default();
    let cwd = std::env::current_dir()
        .and_then(|p| p.canonicalize())
        .unwrap_or_else(|_| PathBuf::from("."));
    let ws_lockfile = PluginLockfile::load_or_default(&cwd.join(".astrid").join(LOCKFILE_NAME))
        .unwrap_or_default();

    println!("{}", Theme::header("Installed Plugins (static)"));
    println!("  {:<20} {:<10} {:<12} SOURCE", "ID", "VERSION", "TYPE");
    println!("{}", Theme::separator());
    let manifests: Vec<_> = discovered.iter().map(|(m, _)| m).collect();
    for m in &manifests {
        let entry_type = match &m.entry_point {
            PluginEntryPoint::Wasm { .. } => "wasm",
            PluginEntryPoint::Mcp { .. } => "mcp",
        };
        let source = lockfile
            .get(&m.id)
            .or_else(|| ws_lockfile.get(&m.id))
            .map_or_else(|| "unknown".to_string(), |e| e.source.to_string());

        println!(
            "  {:<20} {:<10} {:<12} {}",
            m.id,
            m.version,
            entry_type,
            Theme::dimmed(&source)
        );
    }
    println!(
        "\n{}",
        Theme::dimmed(&format!("{} plugin(s)", manifests.len()))
    );

    Ok(())
}

/// Compile a plugin without loading it.
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

/// Remove an installed plugin.
pub(crate) async fn remove_plugin(id: &str) -> anyhow::Result<()> {
    let plugin_id = PluginId::new(id).context("invalid plugin ID")?;
    let id = plugin_id.as_str();
    let home = AstridHome::resolve()?;

    // Find where the plugin is installed
    let plugin_dir = find_plugin_dir(&home, id)?;

    println!("{}", Theme::info(&format!("Removing plugin: {id}")));

    // Best-effort daemon unload
    notify_daemon("unload", id).await;

    // Remove from lockfiles (both user-level and workspace-level).
    // Uses transactional update to hold the exclusive lock across
    // load+mutate+save, preventing TOCTOU races with concurrent installs.
    // Lockfile updates must succeed before we delete plugin files to avoid
    // dangling entries that trigger perpetual integrity violations.
    let cwd = std::env::current_dir()
        .context("failed to get current directory")?
        .canonicalize()
        .context("failed to canonicalize current directory")?;
    // Update workspace lockfile first (more likely to fail — may not exist),
    // then user lockfile (always present). This ordering ensures that if the
    // workspace update fails, the user lockfile hasn't been modified yet.
    let user_lockfile = home.root().join(LOCKFILE_NAME);
    let ws_lockfile = cwd.join(".astrid").join(LOCKFILE_NAME);

    // Only update workspace lockfile if .astrid/ already exists — avoids
    // creating artifacts in directories that never had a workspace lockfile.
    if ws_lockfile.parent().is_some_and(std::path::Path::exists) {
        let pid = plugin_id.clone();
        if let Err(e) = PluginLockfile::update(&ws_lockfile, |lockfile| {
            lockfile.remove(&pid);
            Ok(())
        }) {
            bail!(
                "Failed to update lockfile {} — aborting removal to avoid dangling entries: {e}",
                ws_lockfile.display()
            );
        }
    }

    {
        let pid = plugin_id.clone();
        if let Err(e) = PluginLockfile::update(&user_lockfile, |lockfile| {
            lockfile.remove(&pid);
            Ok(())
        }) {
            bail!(
                "Failed to update lockfile {} — aborting removal to avoid dangling entries: {e}",
                user_lockfile.display()
            );
        }
    }

    // Delete plugin directory (safe now — lockfile entries are already removed)
    std::fs::remove_dir_all(&plugin_dir)
        .with_context(|| format!("failed to remove {}", plugin_dir.display()))?;

    println!("{}", Theme::success(&format!("Plugin '{id}' removed")));
    Ok(())
}

/// Install a plugin from a local path, registry, or git source.
pub(crate) async fn install_plugin(
    source: &str,
    from_openclaw: bool,
    workspace: bool,
) -> anyhow::Result<()> {
    let home = AstridHome::resolve()?;

    // Detect git source prefixes
    if source.starts_with("github:") || source.starts_with("git:") {
        return install_from_git(source, workspace, &home).await;
    }

    if from_openclaw {
        install_from_openclaw(source, workspace, &home).await
    } else {
        install_from_local(source, workspace, &home).await
    }
}

/// Install from the `OpenClaw` npm registry.
///
/// Uses a staging directory for atomicity: compile into a temp dir on the
/// same filesystem, rename into place, then update lockfile. If the lockfile
/// update fails, the rename is rolled back.
async fn install_from_openclaw(
    source: &str,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    println!(
        "{}",
        Theme::info(&format!("Installing from OpenClaw registry: {source}"))
    );

    // Parse npm spec
    let spec = NpmSpec::parse(source).context("invalid package specifier")?;

    // Fetch from npm
    println!(
        "{}",
        Theme::dimmed(&format!("  Fetching {}...", spec.full_name()))
    );
    let fetcher = NpmFetcher::new().context("failed to initialize HTTP client")?;
    let pkg = fetcher.fetch(&spec).await.context("npm fetch failed")?;

    println!(
        "{}",
        Theme::dimmed(&format!("  Resolved {} v{}", pkg.name, pkg.version))
    );

    // Parse the OpenClaw manifest from the extracted package
    let oc_manifest = openclaw_bridge::manifest::parse_manifest(&pkg.package_root)
        .context("fetched package is not a valid OpenClaw plugin")?;

    let astrid_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
        .context("failed to convert OpenClaw ID")?;

    // Detect runtime tier
    let tier = detect_tier(&pkg.package_root, Some(&oc_manifest));
    println!("{}", Theme::dimmed(&format!("  Detected tier: {tier}")));

    let target_dir = resolve_target_dir(home, &astrid_id, workspace)?;

    // Ensure parent exists for staging dir (same filesystem for atomic rename)
    let parent = target_dir.parent().context("target dir has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    // Compile into a staging directory
    let staging = tempfile::tempdir_in(parent).context("failed to create staging directory")?;

    match tier {
        PluginTier::Wasm => {
            println!(
                "{}",
                Theme::dimmed(&format!("  Compiling to WASM (ID: {astrid_id})..."))
            );
            compile_openclaw(&pkg.package_root, staging.path(), home, &oc_manifest)?;
        },
        PluginTier::Node => {
            println!(
                "{}",
                Theme::dimmed(&format!(
                    "  Preparing Tier 2 Node.js bridge (ID: {astrid_id})..."
                ))
            );
            prepare_tier2(&pkg.package_root, staging.path(), home, &oc_manifest)?;
        },
    }

    // Prepare lockfile entry from staging contents before we move anything.
    let lockfile_path = resolve_lockfile_path(home, workspace)?;
    let manifest = load_manifest(&staging.path().join("plugin.toml"))?;
    let source_str = format!("{}@{}", spec.full_name(), pkg.version);
    let locked = LockedPlugin::from_manifest(
        &manifest,
        staging.path(),
        PluginSource::OpenClaw(source_str),
    )?;

    atomic_install(staging, &target_dir, &locked, &lockfile_path)?;

    // Notify daemon
    notify_daemon("load", &astrid_id).await;

    println!(
        "{}",
        Theme::success(&format!("Installed plugin '{astrid_id}'"))
    );
    if workspace {
        println!("{}", Theme::dimmed("  Location: .astrid/plugins/"));
    } else {
        println!(
            "{}",
            Theme::dimmed(&format!("  Location: {}", target_dir.display()))
        );
    }

    Ok(())
}

/// Install from a git repository (GitHub shorthand or generic git URL).
///
/// Fetches the repository, detects the plugin type, compiles if needed,
/// and installs atomically.
async fn install_from_git(source: &str, workspace: bool, home: &AstridHome) -> anyhow::Result<()> {
    let git_source = GitSource::parse(source).context("invalid git source specifier")?;

    println!(
        "{}",
        Theme::info(&format!(
            "Installing from git: {}",
            git_source.display_source()
        ))
    );

    // Fetch the source
    println!("{}", Theme::dimmed("  Fetching repository..."));
    let (_tmp_dir, source_root) = astrid_plugins::git_install::fetch_git_source(&git_source)
        .await
        .context("failed to fetch git source")?;

    // Detect plugin type and route to appropriate pipeline
    if source_root.join("plugin.toml").exists() {
        // Pre-compiled plugin — copy into staging and install
        let manifest = load_manifest(&source_root.join("plugin.toml"))
            .context("failed to load plugin manifest from git source")?;
        let id = manifest.id.as_str().to_string();
        let target_dir = resolve_target_dir(home, &id, workspace)?;

        let parent = target_dir.parent().context("target dir has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let staging = tempfile::tempdir_in(parent).context("failed to create staging directory")?;

        println!(
            "{}",
            Theme::dimmed(&format!("  Installing plugin '{id}'..."))
        );
        copy_plugin_dir(&source_root, staging.path())?;

        let lockfile_path = resolve_lockfile_path(home, workspace)?;
        let locked = LockedPlugin::from_manifest(
            &manifest,
            staging.path(),
            PluginSource::Git {
                url: git_source.display_source(),
                commit: None,
            },
        )?;

        atomic_install(staging, &target_dir, &locked, &lockfile_path)?;
        notify_daemon("load", &id).await;

        println!("{}", Theme::success(&format!("Installed plugin '{id}'")));
    } else if source_root.join("openclaw.plugin.json").exists() {
        // OpenClaw plugin — compile to WASM via staging
        let oc_manifest = openclaw_bridge::manifest::parse_manifest(&source_root)
            .context("failed to parse openclaw.plugin.json from git source")?;
        let astrid_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
            .context("failed to convert OpenClaw ID")?;
        let target_dir = resolve_target_dir(home, &astrid_id, workspace)?;

        let parent = target_dir.parent().context("target dir has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let staging = tempfile::tempdir_in(parent).context("failed to create staging directory")?;

        println!(
            "{}",
            Theme::dimmed(&format!("  Compiling OpenClaw plugin (ID: {astrid_id})..."))
        );
        compile_openclaw(&source_root, staging.path(), home, &oc_manifest)?;

        let lockfile_path = resolve_lockfile_path(home, workspace)?;
        let manifest = load_manifest(&staging.path().join("plugin.toml"))?;
        let locked = LockedPlugin::from_manifest(
            &manifest,
            staging.path(),
            PluginSource::Git {
                url: git_source.display_source(),
                commit: None,
            },
        )?;

        atomic_install(staging, &target_dir, &locked, &lockfile_path)?;
        notify_daemon("load", &astrid_id).await;

        println!(
            "{}",
            Theme::success(&format!("Installed plugin '{astrid_id}'"))
        );
    } else {
        bail!(
            "Cannot detect plugin type in git source '{}'. Expected:\n\
             - plugin.toml (pre-compiled plugin)\n\
             - openclaw.plugin.json (OpenClaw plugin)",
            git_source.display_source()
        );
    }

    if workspace {
        println!("{}", Theme::dimmed("  Location: .astrid/plugins/"));
    }

    Ok(())
}

/// Install from a local path.
///
/// Uses a staging directory for atomicity: copy/compile into a temp dir,
/// rename into place, then update lockfile. If the lockfile update fails,
/// the rename is rolled back.
async fn install_from_local(
    source: &str,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    let source_path = Path::new(source);
    if !source_path.exists() {
        bail!("Source path does not exist: {source}");
    }

    // Canonicalize for reproducible lockfile entries (no relative paths).
    let canonical_source = source_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {source}"))?;

    println!(
        "{}",
        Theme::info(&format!("Installing from local path: {source}"))
    );

    if source_path.join("plugin.toml").exists() {
        // Pre-compiled plugin — copy the directory via staging
        let manifest = load_manifest(&source_path.join("plugin.toml"))
            .context("failed to load plugin manifest")?;
        let id = manifest.id.as_str().to_string();
        let target_dir = resolve_target_dir(home, &id, workspace)?;

        let parent = target_dir.parent().context("target dir has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let staging = tempfile::tempdir_in(parent).context("failed to create staging directory")?;

        println!("{}", Theme::dimmed(&format!("  Copying plugin '{id}'...")));
        copy_plugin_dir(source_path, staging.path())?;

        // Prepare lockfile entry from staging contents.
        let lockfile_path = resolve_lockfile_path(home, workspace)?;
        let locked = LockedPlugin::from_manifest(
            &manifest,
            staging.path(),
            PluginSource::Local(canonical_source.display().to_string()),
        )?;

        atomic_install(staging, &target_dir, &locked, &lockfile_path)?;

        // Notify daemon
        notify_daemon("load", &id).await;

        println!("{}", Theme::success(&format!("Installed plugin '{id}'")));
    } else if source_path.join("openclaw.plugin.json").exists() {
        // OpenClaw plugin directory — needs compilation via staging
        let oc_manifest = openclaw_bridge::manifest::parse_manifest(source_path)
            .context("failed to parse openclaw.plugin.json")?;
        let astrid_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
            .context("failed to convert OpenClaw ID")?;

        // Detect runtime tier
        let tier = detect_tier(source_path, Some(&oc_manifest));
        println!("{}", Theme::dimmed(&format!("  Detected tier: {tier}")));

        let target_dir = resolve_target_dir(home, &astrid_id, workspace)?;

        let parent = target_dir.parent().context("target dir has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let staging = tempfile::tempdir_in(parent).context("failed to create staging directory")?;

        match tier {
            PluginTier::Wasm => {
                println!(
                    "{}",
                    Theme::dimmed(&format!("  Compiling OpenClaw plugin (ID: {astrid_id})..."))
                );
                compile_openclaw(source_path, staging.path(), home, &oc_manifest)?;
            },
            PluginTier::Node => {
                println!(
                    "{}",
                    Theme::dimmed(&format!(
                        "  Preparing Tier 2 Node.js bridge (ID: {astrid_id})..."
                    ))
                );
                prepare_tier2(source_path, staging.path(), home, &oc_manifest)?;
            },
        }

        // Prepare lockfile entry from staging contents.
        let lockfile_path = resolve_lockfile_path(home, workspace)?;
        let manifest = load_manifest(&staging.path().join("plugin.toml"))?;
        let locked = LockedPlugin::from_manifest(
            &manifest,
            staging.path(),
            PluginSource::Local(canonical_source.display().to_string()),
        )?;

        atomic_install(staging, &target_dir, &locked, &lockfile_path)?;

        // Notify daemon
        notify_daemon("load", &astrid_id).await;

        println!(
            "{}",
            Theme::success(&format!("Installed plugin '{astrid_id}'"))
        );
    } else {
        bail!(
            "Cannot detect plugin type at '{source}'. Expected:\n\
             - Directory with plugin.toml (pre-compiled plugin)\n\
             - Directory with openclaw.plugin.json (OpenClaw plugin)"
        );
    }

    if workspace {
        println!("{}", Theme::dimmed("  Location: .astrid/plugins/"));
    }

    Ok(())
}
