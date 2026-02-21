//! Plugin management commands - install, remove, list, compile, and inspect plugins.

use anyhow::Context;

use astrid_core::dirs::AstridHome;
use astrid_plugins::load_manifest;
use astrid_plugins::lockfile::{LOCKFILE_NAME, LockedPlugin, PluginLockfile};
use astrid_plugins::manifest::{PluginCapability, PluginEntryPoint};
use astrid_plugins::plugin::PluginId;

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Best-effort daemon notification. Prints a warning on failure, never fails the command.

/// Show detailed information about an installed plugin.

/// List installed plugins with state information.
///
/// Connects to the daemon via JSON-RPC if running (live state),
/// falls back to manifest scan (static) otherwise.

/// Compile a plugin without loading it.

/// Remove an installed plugin.

/// Install a plugin from a local path, registry, or git source.

/// Install from the `OpenClaw` npm registry.
///
/// Uses a staging directory for atomicity: compile into a temp dir on the
/// same filesystem, rename into place, then update lockfile. If the lockfile
/// update fails, the rename is rolled back.

/// Install from a git repository (GitHub shorthand or generic git URL).
///
/// Fetches the repository, detects the plugin type, compiles if needed,
/// and installs atomically.

/// Install from a local path.
///
/// Uses a staging directory for atomicity: copy/compile into a temp dir,
/// rename into place, then update lockfile. If the lockfile update fails,
/// the rename is rolled back.
use super::helpers::find_plugin_dir;
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
                PluginCapability::Connector { profile } => {
                    format!("connector (profile: {profile})")
                },
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
