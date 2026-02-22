//! Plugin management commands - install, remove, list, compile, and inspect plugins.

use anyhow::{Context, bail};

use astrid_core::dirs::AstridHome;
use astrid_plugins::lockfile::{LOCKFILE_NAME, PluginLockfile};
use astrid_plugins::plugin::PluginId;

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use super::helpers::{find_plugin_dir, notify_daemon};
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
