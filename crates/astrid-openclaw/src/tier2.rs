//! Tier 2 (Node.js MCP) manifest generation and build helpers.
//!
//! Contains the `Capsule.toml` schema types, Node.js binary resolution,
//! source tree copying, and `TypeScript` transpilation for Tier 2 plugins.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{BridgeError, BridgeResult};
use crate::manifest::{self, OpenClawManifest};
use crate::transpiler;

// ── Capsule.toml schema types ──────────────────────────────────────────

/// Serializable Tier 2 `Capsule.toml` manifest.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Tier2Manifest {
    pub(crate) package: Tier2Package,
    #[serde(default, rename = "uplink", skip_serializing_if = "Vec::is_empty")]
    pub(crate) uplinks: Vec<Tier2UplinkDef>,
    pub(crate) mcp_server: Vec<Tier2McpServer>,
    pub(crate) capabilities: Tier2Capabilities,
    #[serde(default, skip_serializing_if = "Tier2Dependencies::is_empty")]
    pub(crate) dependencies: Tier2Dependencies,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub(crate) env: HashMap<String, Tier2EnvDef>,
}

/// Capability-based dependency declarations for Tier 2 capsule manifests.
#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct Tier2Dependencies {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) provides: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) requires: Vec<String>,
}

impl Tier2Dependencies {
    pub(crate) fn is_empty(&self) -> bool {
        self.provides.is_empty() && self.requires.is_empty()
    }
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct Tier2Package {
    pub(crate) name: String,
    pub(crate) version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct Tier2McpServer {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) server_type: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_false(v: &bool) -> bool {
    !v
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct Tier2Capabilities {
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) uplink: bool,
    pub(crate) host_process: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct Tier2UplinkDef {
    pub(crate) name: String,
    pub(crate) platform: String,
    pub(crate) profile: String,
}

pub(crate) fn channel_to_platform(channel: &str) -> String {
    channel.to_lowercase()
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct Tier2EnvDef {
    #[serde(rename = "type")]
    pub(crate) env_type: String,
    pub(crate) request: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) default: Option<String>,
    #[serde(rename = "enum", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) enum_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) placeholder: Option<String>,
}

// ── Manifest generation ────────────────────────────────────────────────

/// Build the `[env]` map for a Tier 2 manifest from `configSchema` + `uiHints`.
pub(crate) fn build_tier2_env(
    oc_manifest: &OpenClawManifest,
) -> BridgeResult<HashMap<String, Tier2EnvDef>> {
    let mut env = HashMap::new();
    for (key, f) in manifest::extract_env_fields(oc_manifest)? {
        env.insert(
            key,
            Tier2EnvDef {
                env_type: f.env_type,
                request: f.request,
                description: f.description,
                default: f.default,
                enum_values: f.enum_values,
                placeholder: f.placeholder,
            },
        );
    }
    Ok(env)
}

/// Generate a `Capsule.toml` for Tier 2 plugins using `[[mcp_server]]`.
pub(crate) fn generate_tier2_manifest(
    astrid_id: &str,
    oc_manifest: &OpenClawManifest,
    entry_point_rel: &str,
    output_dir: &Path,
) -> BridgeResult<()> {
    let env = build_tier2_env(oc_manifest)?;

    let uplinks: Vec<Tier2UplinkDef> = oc_manifest
        .channels
        .iter()
        .map(|ch| Tier2UplinkDef {
            name: ch.clone(),
            platform: channel_to_platform(ch),
            profile: "bridge".to_string(),
        })
        .collect();

    let manifest = Tier2Manifest {
        package: Tier2Package {
            name: astrid_id.to_string(),
            version: oc_manifest.display_version().to_string(),
            description: oc_manifest.description.clone(),
        },
        uplinks,
        mcp_server: vec![Tier2McpServer {
            id: astrid_id.to_string(),
            server_type: "stdio".to_string(),
            command: resolve_node_binary(),
            args: vec![
                "astrid_bridge.mjs".to_string(),
                "--entry".to_string(),
                entry_point_rel.to_string(),
                "--plugin-id".to_string(),
                astrid_id.to_string(),
            ],
        }],
        capabilities: Tier2Capabilities {
            uplink: !oc_manifest.channels.is_empty(),
            host_process: vec![resolve_node_binary()],
        },
        dependencies: {
            let mut provides = Vec::new();
            for channel in &oc_manifest.channels {
                if channel.is_empty() || channel.split('.').any(str::is_empty) {
                    return Err(BridgeError::Manifest(format!(
                        "channel name '{channel}' is invalid (empty or contains empty segments)"
                    )));
                }
                provides.push(format!("uplink:{channel}"));
            }
            for provider in &oc_manifest.providers {
                if provider.is_empty() || provider.split('.').any(str::is_empty) {
                    return Err(BridgeError::Manifest(format!(
                        "provider name '{provider}' is invalid (empty or contains empty segments)"
                    )));
                }
                provides.push(format!("llm:{provider}"));
            }
            Tier2Dependencies {
                provides,
                ..Default::default()
            }
        },
        env,
    };

    let toml_content = toml::to_string_pretty(&manifest)
        .map_err(|e| BridgeError::Output(format!("failed to serialize Capsule.toml: {e}")))?;

    let toml_path = output_dir.join("Capsule.toml");
    std::fs::write(&toml_path, toml_content)
        .map_err(|e| BridgeError::Output(format!("failed to write Capsule.toml: {e}")))?;

    Ok(())
}

// ── Node.js resolution ─────────────────────────────────────────────────

/// Resolve the best available `Node.js` binary (>= 22).
///
/// Prefers versioned Homebrew installs (`node@22`, `node@23`, …) over the
/// default `node` on `PATH`, since `OpenClaw` plugins require Node >= 22 for
/// native `TypeScript` imports. Each candidate is executed with `--version`
/// to verify it actually works (broken dylibs, etc.). Falls back to
/// `"node"` if nothing better is found.
pub(crate) fn resolve_node_binary() -> String {
    /// Run `<binary> --version` and return the major version if successful.
    fn node_major(bin: &str) -> Option<u32> {
        let output = std::process::Command::new(bin)
            .arg("--version")
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let version = String::from_utf8(output.stdout).ok()?;
        version
            .trim()
            .strip_prefix('v')
            .and_then(|s| s.split('.').next())
            .and_then(|s| s.parse().ok())
    }

    // Check versioned Homebrew installs (highest version first).
    // Apple Silicon uses /opt/homebrew, Intel Macs use /usr/local.
    for prefix in ["/opt/homebrew", "/usr/local"] {
        for v in (22..=26).rev() {
            let path = format!("{prefix}/opt/node@{v}/bin/node");
            if node_major(&path).is_some_and(|m| m >= 22) {
                return path;
            }
        }
    }
    // Check if default `node` meets the minimum version
    if node_major("node").is_some_and(|m| m >= 22) {
        return "node".to_string();
    }
    // Fallback — let the OS resolve it at runtime
    "node".to_string()
}

// ── Source tree helpers ─────────────────────────────────────────────────

/// Maximum nesting depth for plugin source tree traversal.
const MAX_COPY_DEPTH: usize = 64;

/// Copy plugin source files, skipping `node_modules`, `.git`, etc.
pub(crate) fn copy_plugin_source(src: &Path, dst: &Path, depth: usize) -> BridgeResult<()> {
    if depth > MAX_COPY_DEPTH {
        return Err(BridgeError::Manifest(
            "plugin source tree exceeds maximum nesting depth (64)".into(),
        ));
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip build artifacts and large directories
        if matches!(
            name_str.as_ref(),
            "node_modules"
                | ".git"
                | "dist"
                | "target"
                | ".next"
                | ".nuxt"
                | ".turbo"
                | "build"
                | ".cache"
                | ".parcel-cache"
                | ".yarn"
        ) {
            continue;
        }

        let dst_path = dst.join(&name);

        if file_type.is_symlink() {
            return Err(BridgeError::Manifest(format!(
                "plugin source contains a symlink at {} — symlinks are not permitted in capsule archives",
                entry.path().display()
            )));
        }

        if file_type.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copy_plugin_source(&entry.path(), &dst_path, depth.saturating_add(1))?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

/// Walk a directory tree and transpile all `.ts`/`.tsx` files to `.js`.
///
/// Skips `node_modules` and dotfiles. Leaves the original `.ts` files in
/// place — the generated `.js` files satisfy the import specifiers that
/// `TypeScript` projects conventionally use (e.g. `import ... from "./foo.js"`).
pub(crate) fn transpile_ts_tree(dir: &Path) -> BridgeResult<()> {
    transpile_ts_tree_inner(dir, 0)
}

fn transpile_ts_tree_inner(dir: &Path, depth: usize) -> BridgeResult<()> {
    if depth > MAX_COPY_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "node_modules" || name_str.starts_with('.') {
            continue;
        }

        let ft = entry.file_type()?;
        if ft.is_dir() {
            transpile_ts_tree_inner(&entry.path(), depth.saturating_add(1))?;
        } else if ft.is_file()
            && (name_str.ends_with(".ts") || name_str.ends_with(".tsx"))
            && !name_str.ends_with(".d.ts")
        {
            let source = std::fs::read_to_string(entry.path())?;
            let js = transpiler::strip_types(&source, &name_str)?;

            // Write .js alongside .ts
            let js_path = entry.path().with_extension("js");
            std::fs::write(&js_path, js)?;
        }
    }
    Ok(())
}
