//! Plugin management commands - install, remove, list, compile, and inspect plugins.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use astralis_core::dirs::AstralisHome;
use astralis_plugins::lockfile::{LOCKFILE_NAME, LockedPlugin, PluginLockfile, PluginSource};
use astralis_plugins::manifest::{PluginCapability, PluginEntryPoint};
use astralis_plugins::npm::{NpmFetcher, NpmSpec};
use astralis_plugins::plugin::PluginId;
use astralis_plugins::{discover_manifests, load_manifest};

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine the target installation directory for a plugin.
fn resolve_target_dir(home: &AstralisHome, id: &str, workspace: bool) -> anyhow::Result<PathBuf> {
    if workspace {
        let cwd = std::env::current_dir()
            .context("failed to get current directory")?
            .canonicalize()
            .context("failed to canonicalize current directory")?;
        Ok(cwd.join(".astralis/plugins").join(id))
    } else {
        Ok(home.plugins_dir().join(id))
    }
}

/// Determine the lockfile path (user-level or workspace-level).
fn resolve_lockfile_path(home: &AstralisHome, workspace: bool) -> anyhow::Result<PathBuf> {
    if workspace {
        let cwd = std::env::current_dir()
            .context("failed to get current directory")?
            .canonicalize()
            .context("failed to canonicalize current directory")?;
        Ok(cwd.join(".astralis").join(LOCKFILE_NAME))
    } else {
        Ok(home.root().join(LOCKFILE_NAME))
    }
}

/// Find an installed plugin directory by checking user-level then workspace-level.
fn find_plugin_dir(home: &AstralisHome, id: &str) -> anyhow::Result<PathBuf> {
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
    let ws_dir = cwd.join(".astralis/plugins").join(id);
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
/// Returns the Astralis plugin ID derived from the `OpenClaw` manifest.
fn compile_openclaw(
    source_dir: &Path,
    output_dir: &Path,
    home: &AstralisHome,
) -> anyhow::Result<String> {
    let oc_manifest = openclaw_bridge::manifest::parse_manifest(source_dir)
        .context("failed to parse openclaw.plugin.json")?;

    let astralis_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
        .context("failed to convert OpenClaw ID to Astralis ID")?;

    let main_path = source_dir.join(&oc_manifest.main);
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
        return Ok(astralis_id);
    }

    // Full pipeline: transpile → shim → compile → generate_manifest
    let js = openclaw_bridge::transpiler::transpile(&raw_source, &oc_manifest.main)
        .context("transpilation failed")?;

    let config: HashMap<String, serde_json::Value> = HashMap::new();
    let shimmed = openclaw_bridge::shim::generate(&js, &config);

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let wasm_path = output_dir.join("plugin.wasm");
    openclaw_bridge::compiler::compile(&shimmed, &wasm_path).context("WASM compilation failed")?;

    openclaw_bridge::output::generate_manifest(
        &astralis_id,
        &oc_manifest,
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

    Ok(astralis_id)
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
    let home = AstralisHome::resolve()?;

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
        cwd.join(".astralis").join(LOCKFILE_NAME),
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
    let home = AstralisHome::resolve()?;
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
    let ws_lockfile = PluginLockfile::load_or_default(&cwd.join(".astralis").join(LOCKFILE_NAME))
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

    let home = AstralisHome::resolve()?;

    // Detect source type
    if source_path.is_dir() && source_path.join("openclaw.plugin.json").exists() {
        // OpenClaw plugin directory
        let out_dir = output.map_or_else(|| source_path.join("dist"), PathBuf::from);

        println!(
            "{}",
            Theme::info(&format!("Compiling OpenClaw plugin at: {path}"))
        );
        let astralis_id = compile_openclaw(source_path, &out_dir, &home)?;
        let wasm_path = out_dir.join("plugin.wasm");
        let meta = std::fs::metadata(&wasm_path)?;
        let hash = LockedPlugin::compute_wasm_hash(&wasm_path)?;

        println!("{}", Theme::success("Compilation complete"));
        println!("{}", Theme::kv("Plugin ID", &astralis_id));
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
    let home = AstralisHome::resolve()?;

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
    let ws_lockfile = cwd.join(".astralis").join(LOCKFILE_NAME);

    // Only update workspace lockfile if .astralis/ already exists — avoids
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

/// Install a plugin from a local path or registry.
pub(crate) async fn install_plugin(
    source: &str,
    from_openclaw: bool,
    workspace: bool,
) -> anyhow::Result<()> {
    let home = AstralisHome::resolve()?;

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
    home: &AstralisHome,
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

    let astralis_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
        .context("failed to convert OpenClaw ID")?;

    let target_dir = resolve_target_dir(home, &astralis_id, workspace)?;

    // Ensure parent exists for staging dir (same filesystem for atomic rename)
    let parent = target_dir.parent().context("target dir has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    // Compile into a staging directory
    let staging = tempfile::tempdir_in(parent).context("failed to create staging directory")?;

    println!(
        "{}",
        Theme::dimmed(&format!("  Compiling to WASM (ID: {astralis_id})..."))
    );

    compile_openclaw(&pkg.package_root, staging.path(), home)?;

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
    notify_daemon("load", &astralis_id).await;

    println!(
        "{}",
        Theme::success(&format!("Installed plugin '{astralis_id}'"))
    );
    if workspace {
        println!("{}", Theme::dimmed("  Location: .astralis/plugins/"));
    } else {
        println!(
            "{}",
            Theme::dimmed(&format!("  Location: {}", target_dir.display()))
        );
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
    home: &AstralisHome,
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
        let astralis_id = openclaw_bridge::manifest::convert_id(&oc_manifest.id)
            .context("failed to convert OpenClaw ID")?;
        let target_dir = resolve_target_dir(home, &astralis_id, workspace)?;

        let parent = target_dir.parent().context("target dir has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let staging = tempfile::tempdir_in(parent).context("failed to create staging directory")?;

        println!(
            "{}",
            Theme::dimmed(&format!(
                "  Compiling OpenClaw plugin (ID: {astralis_id})..."
            ))
        );
        compile_openclaw(source_path, staging.path(), home)?;

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
        notify_daemon("load", &astralis_id).await;

        println!(
            "{}",
            Theme::success(&format!("Installed plugin '{astralis_id}'"))
        );
    } else {
        bail!(
            "Cannot detect plugin type at '{source}'. Expected:\n\
             - Directory with plugin.toml (pre-compiled plugin)\n\
             - Directory with openclaw.plugin.json (OpenClaw plugin)"
        );
    }

    if workspace {
        println!("{}", Theme::dimmed("  Location: .astralis/plugins/"));
    }

    Ok(())
}
