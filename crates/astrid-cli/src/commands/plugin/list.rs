//! Plugin management commands - install, remove, list, compile, and inspect plugins.

use std::path::PathBuf;

use astrid_core::dirs::AstridHome;
use astrid_plugins::discover_manifests;
use astrid_plugins::lockfile::{LOCKFILE_NAME, PluginLockfile};
use astrid_plugins::manifest::PluginEntryPoint;

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
