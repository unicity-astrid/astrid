//! Plugin management commands - install, remove, list, compile, and inspect plugins.

use std::path::PathBuf;

use anyhow::{Context, bail};

use astrid_core::dirs::AstridHome;
use astrid_plugins::lockfile::LOCKFILE_NAME;

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine the target installation directory for a plugin.
pub(crate) fn resolve_target_dir(
    home: &AstridHome,
    id: &str,
    workspace: bool,
) -> anyhow::Result<PathBuf> {
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
pub(crate) fn resolve_lockfile_path(home: &AstridHome, workspace: bool) -> anyhow::Result<PathBuf> {
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
pub(crate) fn find_plugin_dir(home: &AstridHome, id: &str) -> anyhow::Result<PathBuf> {
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
pub(crate) async fn notify_daemon(action: &str, plugin_id: &str) {
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
