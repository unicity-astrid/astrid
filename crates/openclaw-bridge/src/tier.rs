//! Tier detection for `OpenClaw` plugins.
//!
//! Determines whether an `OpenClaw` plugin should run as:
//! - **Tier 1 (WASM)**: Single-file plugins without npm dependencies
//! - **Tier 2 (Node.js MCP)**: Plugins with npm dependencies, channels,
//!   providers, or unsupported runtime features
//!
//! Detection order:
//! 1. Manifest declares `channels` or `providers` → Node (requires host integration)
//! 2. Presence of `package.json` with non-empty `dependencies` → Node
//! 3. Source imports of unsupported `node:*` modules → Node
//! 4. Source imports local relative paths (`./`, `../`) → Node (multi-file plugin)
//! 5. Default: Tier 1 (WASM)

use std::path::Path;

use crate::manifest::OpenClawManifest;

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
    "node:http",
    "node:https",
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
    "http",
    "https",
    "net",
    "child_process",
    "worker_threads",
    "cluster",
    "dgram",
    "tls",
    "http2",
    "inspector",
    "v8",
    "vm",
    "async_hooks",
];

/// Detect the appropriate runtime tier for an `OpenClaw` plugin.
///
/// If the manifest has already been parsed, pass it to avoid redundant I/O.
/// `plugin_dir` should contain `openclaw.plugin.json` and optionally `package.json`.
///
/// Detection order:
/// 1. Manifest declares `channels` or `providers` → Node (host integration required)
/// 2. `package.json` with non-empty `"dependencies"` → Node
/// 3. Source files import unsupported `node:*` modules → Node
/// 4. Source files import local relative paths (`./`, `../`) → Node (multi-file)
/// 5. Default → Wasm
#[must_use]
pub fn detect_tier(plugin_dir: &Path, manifest: Option<&OpenClawManifest>) -> PluginTier {
    // 1. Check for channels/providers in manifest (requires host integration)
    let needs_host = if let Some(m) = manifest {
        m.requires_host_integration()
    } else {
        requires_host_integration_from_file(plugin_dir)
    };
    if needs_host {
        return PluginTier::Node;
    }

    // 2. Check for package.json with dependencies
    if has_npm_dependencies(plugin_dir) {
        return PluginTier::Node;
    }

    // 3. Check source for unsupported node:* imports
    if has_unsupported_imports(plugin_dir) {
        return PluginTier::Node;
    }

    // 4. Check for local relative imports (multi-file plugin)
    if has_local_imports(plugin_dir) {
        return PluginTier::Node;
    }

    // 5. Default to WASM
    PluginTier::Wasm
}

/// Fallback: read `openclaw.plugin.json` from disk when no parsed manifest is available.
fn requires_host_integration_from_file(plugin_dir: &Path) -> bool {
    let manifest_path = plugin_dir.join("openclaw.plugin.json");
    let Ok(content) = std::fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };

    let has_channels = parsed
        .get("channels")
        .and_then(|c| c.as_array())
        .is_some_and(|arr| !arr.is_empty());

    let has_providers = parsed
        .get("providers")
        .and_then(|p| p.as_array())
        .is_some_and(|arr| !arr.is_empty());

    has_channels || has_providers
}

/// Check if `package.json` exists and has non-empty `dependencies`.
///
/// Note: only checks `"dependencies"`, not `"peerDependencies"`. Plugins
/// using peer deps for runtime requirements may be mis-classified as Tier 1.
/// In practice `OpenClaw` plugins use `"dependencies"` for runtime deps.
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

/// Scan the entry point file for imports of unsupported Node.js modules.
///
/// This is a **heuristic** — it uses substring matching on the entry point file only.
/// False positives (e.g. module name in a comment) are safe (plugin still works via Node).
/// False negatives (e.g. unsupported import in a transitive dependency) may cause WASM
/// compilation to fail; the user can work around this by adding npm `dependencies` to
/// `package.json` which triggers the earlier Tier 2 check.
///
/// Resolves the entry point via `package.json` → `openclaw.extensions`
/// or falls back to common file locations.
fn has_unsupported_imports(plugin_dir: &Path) -> bool {
    // Use the manifest entry point resolver
    let entry_path = match crate::manifest::resolve_entry_point(plugin_dir) {
        Ok(entry) => plugin_dir.join(entry),
        Err(_) => return false,
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

/// Scan the entry point file for local relative imports (`./` or `../`).
///
/// Multi-file plugins cannot be compiled to a single WASM module — they must
/// run via the Node.js subprocess bridge. Like `has_unsupported_imports`, this
/// is a heuristic on the entry point only; false positives are safe.
fn has_local_imports(plugin_dir: &Path) -> bool {
    let entry_path = match crate::manifest::resolve_entry_point(plugin_dir) {
        Ok(entry) => plugin_dir.join(entry),
        Err(_) => return false,
    };

    let Ok(source) = std::fs::read_to_string(entry_path) else {
        return false;
    };

    // Match import/require of relative paths: "./foo" or "../bar"
    for line in source.lines() {
        let trimmed = line.trim();
        // Skip comments
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        if (trimmed.contains("from \"./")
            || trimmed.contains("from \"../")
            || trimmed.contains("from './")
            || trimmed.contains("from '../")
            || trimmed.contains("require(\"./")
            || trimmed.contains("require(\"../")
            || trimmed.contains("require('./")
            || trimmed.contains("require('../"))
            && (trimmed.starts_with("import") || trimmed.contains("require("))
        {
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
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Wasm);
    }

    #[test]
    fn channels_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "discord", "configSchema": {}, "channels": ["discord"]}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn providers_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "copilot", "configSchema": {}, "providers": ["copilot-proxy"]}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn empty_channels_stays_wasm() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "test", "configSchema": {}, "channels": []}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Wasm);
    }

    #[test]
    fn package_json_with_deps_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = r#"{"name": "test", "dependencies": {"nostr-tools": "^2.0.0"}}"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn package_json_empty_deps_stays_wasm() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = r#"{"name": "test", "dependencies": {}}"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Wasm);
    }

    #[test]
    fn unsupported_import_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        // Need package.json with openclaw.extensions for entry point resolution
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"openclaw":{"extensions":["./src/index.ts"]}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let source = r#"import { createServer } from "node:net";"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn unsupported_import_fallback_entry() {
        let dir = tempfile::tempdir().unwrap();
        // No package.json — fallback to src/index.ts
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let source = r#"import { createServer } from "node:net";"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn local_import_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let source = r#"
import { config } from "./config.js";
import { sendMessage } from "./tools/send-message.js";
"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn local_import_parent_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let source = r#"import { shared } from "../shared/utils.js";"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn local_require_triggers_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let source = r#"const config = require("./config");"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Node);
    }

    #[test]
    fn comment_with_local_import_stays_wasm() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let source = r#"
// import { config } from "./config.js";
export function main() { return "hello"; }
"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Wasm);
    }

    #[test]
    fn polyfilled_import_stays_wasm() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        // node:fs and node:path are polyfilled — should NOT trigger Node tier
        let source = r#"
import { readFileSync } from "node:fs";
import { join } from "node:path";
"#;
        std::fs::write(dir.path().join("src/index.ts"), source).unwrap();
        assert_eq!(detect_tier(dir.path(), None), PluginTier::Wasm);
    }
}
