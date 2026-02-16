//! Embedded Node.js MCP bridge for Tier 2 plugins.
//!
//! The bridge script (`astrid_bridge.mjs`) is embedded at compile time via
//! `include_str!()`. At install time the script is written to the plugin
//! directory alongside the transpiled plugin source.

use std::path::Path;

use crate::error::{BridgeError, BridgeResult};

/// The universal MCP bridge script, embedded at compile time.
pub const BRIDGE_SCRIPT: &str = include_str!("../bridge/astrid_bridge.mjs");

/// Write the bridge script to `dest_dir/astrid_bridge.mjs`.
///
/// # Errors
///
/// Returns [`BridgeError::Output`] if the file cannot be written.
pub fn write_bridge_script(dest_dir: &Path) -> BridgeResult<()> {
    let dest = dest_dir.join("astrid_bridge.mjs");
    std::fs::write(&dest, BRIDGE_SCRIPT).map_err(|e| {
        BridgeError::Output(format!(
            "failed to write bridge script to {}: {e}",
            dest.display()
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_script_is_nonempty() {
        assert!(
            !BRIDGE_SCRIPT.is_empty(),
            "embedded bridge script should not be empty"
        );
    }

    #[test]
    fn bridge_script_contains_mcp_handler() {
        assert!(
            BRIDGE_SCRIPT.contains("handleInitialize"),
            "bridge script should contain MCP initialize handler"
        );
        assert!(
            BRIDGE_SCRIPT.contains("handleToolsList"),
            "bridge script should contain tools/list handler"
        );
        assert!(
            BRIDGE_SCRIPT.contains("handleToolsCall"),
            "bridge script should contain tools/call handler"
        );
    }

    #[test]
    fn write_bridge_script_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        write_bridge_script(dir.path()).unwrap();

        let file = dir.path().join("astrid_bridge.mjs");
        assert!(file.exists());

        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, BRIDGE_SCRIPT);
    }
}
