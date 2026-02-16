//! Tier detection for `OpenClaw` plugins.
//!
//! Determines whether an `OpenClaw` plugin should run as:
//! - **Tier 1 (WASM)**: Single-file plugins without npm dependencies
//! - **Tier 2 (Node.js MCP)**: Plugins with npm dependencies or unsupported runtime features
//!
//!
//! Detection order:
//! 1. Explicit `"runtime"` override in `openclaw.plugin.json`
//! 2. Presence of `package.json` with non-empty `dependencies`
//! 3. Source imports of unsupported `node:*` modules
//! 4. Default: Tier 1 (WASM)

use std::path::Path;

/// The runtime tier for a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTier {
    /// Tier 1: compiled to WASM, runs in Extism sandbox.
    Wasm,
    /// Tier 2: runs as a sandboxed Node.js subprocess via MCP bridge.
    Node,
}

impl std::fmt::Display for PluginTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wasm => write!(f, "wasm"),
            Self::Node => write!(f, "node"),
        }
    }
}

/// Node.js built-in modules that cannot be polyfilled in WASM.
///
/// Plugins importing these modules are automatically routed to Tier 2.
/// Modules that *can* be polyfilled (fs, path, os) are NOT in this list.
const UNSUPPORTED_NODE_MODULES: &[&str] = &[
    "node:net",
    "node:child_process",
    "node:worker_threads",
    "node:cluster",
    "node:dgram",
    "node:tls",
    "node:http2",
    "node:inspector",
    "node:v8",
    "node:vm",
    "node:async_hooks",
    "net",
    "child_process",
    "worker_threads",
    "cluster",
    "dgram",
    "tls",
    "http2",
];

/// Detect the appropriate runtime tier for an `OpenClaw` plugin.
///
/// `plugin_dir` should contain `openclaw.plugin.json` and optionally `package.json`.
///
/// Detection order:
/// 1. Explicit `"runtime": "node"` or `"runtime": "wasm"` in `openclaw.plugin.json`
/// 2. `package.json` with non-empty `"dependencies"` → Node
/// 3. Source files import unsupported `node:*` modules → Node
/// 4. Default → Wasm
#[must_use]
pub fn detect_tier(plugin_dir: &Path) -> PluginTier {
    // 1. Check for explicit override in openclaw.plugin.json
    if let Some(tier) = check_manifest_override(plugin_dir) {
        return tier;
    }

    // 2. Check for package.json with dependencies
    if has_npm_dependencies(plugin_dir) {
        return PluginTier::Node;
    }

    // 3. Check source for unsupported node:* imports
    if has_unsupported_imports(plugin_dir) {
        return PluginTier::Node;
    }

    // 4. Default to WASM
    PluginTier::Wasm
}

/// Check `openclaw.plugin.json` for an explicit `"runtime"` field.
fn check_manifest_override(plugin_dir: &Path) -> Option<PluginTier> {
    let manifest_path = plugin_dir.join("openclaw.plugin.json");
    let content = std::fs::read_to_string(manifest_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    let runtime = parsed.get("runtime")?.as_str()?;

    match runtime {
        "node" => Some(PluginTier::Node),
        "wasm" => Some(PluginTier::Wasm),
        _ => None,
    }
}

/// Check if `package.json` exists and has non-empty `dependencies`.
fn has_npm_dependencies(plugin_dir: &Path) -> bool {
    let pkg_path = plugin_dir.join("package.json");
    let Ok(content) = std::fs::read_to_string(pkg_path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };

    parsed
        .get("dependencies")
        .and_then(|d| d.as_object())
        .is_some_and(|deps| !deps.is_empty())
}

/// Scan source files for imports of unsupported Node.js modules.
///
/// Only checks the entry point file referenced in `openclaw.plugin.json`.
/// A full dependency-tree scan would be more thorough but is deferred
/// to avoid expensive I/O during detection.
fn has_unsupported_imports(plugin_dir: &Path) -> bool {
    // Try to find the entry point from the manifest
    let manifest_path = plugin_dir.join("openclaw.plugin.json");
    let entry = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v.get("main")?.as_str().map(String::from));

    let entry_path = if let Some(main) = entry {
        plugin_dir.join(main)
    } else {
        // Fallback: check src/index.ts or src/index.js
        let ts = plugin_dir.join("src/index.ts");
        let js = plugin_dir.join("src/index.js");
        if ts.exists() {
            ts
        } else if js.exists() {
            js
        } else {
            return false;
        }
    };

    let Ok(source) = std::fs::read_to_string(entry_path) else {
        return false;
    };

    for module in UNSUPPORTED_NODE_MODULES {
        // Check for: import ... from "module" or require("module")
        if source.contains(&format!("\"{module}\"")) || source.contains(&format!("'{module}'")) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_tiers() {
        assert_eq!(PluginTier::Wasm.to_string(), "wasm");
        assert_eq!(PluginTier::Node.to_string(), "node");
    }

    #[test]
    fn empty_dir_defaults_to_wasm() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_tier(dir.path()), PluginTier::Wasm);
    }

    #[test]
    fn explicit_node_override() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "test", "main": "src/index.ts", "runtime": "node"}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        assert_eq!(detect_tier(dir.path()), PluginTier::Node);
    }

    #[test]
    fn explicit_wasm_override() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "test", "main": "src/index.ts", "runtime": "wasm"}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        assert_eq!(detect_tier(dir.path()), PluginTier::Wasm);
    }

    #[test]
    fn package_json_with_deps_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = r#"{"name": "test", "dependencies": {"nostr-tools": "^2.0.0"}}"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        assert_eq!(detect_tier(dir.path()), PluginTier::Node);
    }

    #[test]
    fn package_json_empty_deps_stays_wasm() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = r#"{"name": "test", "dependencies": {}}"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        assert_eq!(detect_tier(dir.path()), PluginTier::Wasm);
    }

    #[test]
    fn unsupported_import_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "test", "main": "src/index.ts"}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let source = r#"import { createServer } from "node:net";"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path()), PluginTier::Node);
    }

    #[test]
    fn polyfilled_import_stays_wasm() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "test", "main": "src/index.ts"}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        // node:fs and node:path are polyfilled — should NOT trigger Node tier
        let source = r#"
import { readFileSync } from "node:fs";
import { join } from "node:path";
"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path()), PluginTier::Wasm);
    }
}
