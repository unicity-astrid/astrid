//! Per-plugin data directory management.
//!
//! Each Tier 2 (Node.js MCP bridge) plugin gets an isolated data directory
//! at `~/.astrid/plugin-data/<plugin-id>/`. This directory is set as the
//! plugin's `HOME` environment variable, confining its filesystem writes
//! to a known, sandboxable location.

use std::path::{Path, PathBuf};

/// Return the per-plugin data directory path.
///
/// Layout: `<astrid_root>/plugin-data/<plugin_id>/`
///
/// Does **not** create the directory — call [`ensure_plugin_data_dir`] for that.
#[must_use]
pub fn plugin_data_dir(astrid_root: &Path, plugin_id: &str) -> PathBuf {
    astrid_root.join("plugin-data").join(plugin_id)
}

/// Ensure the per-plugin data directory exists, creating it if necessary.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be created.
pub fn ensure_plugin_data_dir(astrid_root: &Path, plugin_id: &str) -> std::io::Result<PathBuf> {
    let dir = plugin_data_dir(astrid_root, plugin_id);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_data_dir_layout() {
        let root = Path::new("/home/user/.astrid");
        let dir = plugin_data_dir(root, "openclaw-unicity");
        assert_eq!(
            dir,
            PathBuf::from("/home/user/.astrid/plugin-data/openclaw-unicity")
        );
    }

    #[test]
    fn ensure_plugin_data_dir_creates() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = ensure_plugin_data_dir(tmp.path(), "test-plugin").unwrap();
        assert!(dir.exists());
        assert!(dir.is_dir());
        assert_eq!(dir.file_name().unwrap(), "test-plugin");
    }
}
