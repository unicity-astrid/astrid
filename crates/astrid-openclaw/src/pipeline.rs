//! High-level compilation pipeline integrating cache, tier detection, and both
//! Tier 1 (WASM) and Tier 2 (Node.js MCP) paths.
//!
//! This is the primary API for consumers (e.g. `astrid-cli`). Instead of
//! calling `transpiler`, `shim`, `compiler`, `cache`, and `tier` individually,
//! call [`compile_plugin`] and get back a [`CompileResult`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cache::CompilationCache;
use crate::compiler;
use crate::error::{BridgeError, BridgeResult};
use crate::manifest::{self, OpenClawManifest};
use crate::node_bridge;
use crate::output;
use crate::shim;
use crate::tier::{self, PluginTier};
use crate::transpiler;

/// Options for the compilation pipeline.
#[derive(Debug)]
pub struct CompileOptions<'a> {
    /// Path to the `OpenClaw` plugin directory (containing `openclaw.plugin.json`).
    pub plugin_dir: &'a Path,
    /// Output directory for the compiled capsule artifacts.
    pub output_dir: &'a Path,
    /// Plugin configuration values (from `--config` or env var prompts).
    pub config: &'a HashMap<String, serde_json::Value>,
    /// Root directory for the compilation cache. `None` disables caching.
    pub cache_dir: Option<&'a Path>,
    /// If `true`, skip WASM compilation and only emit the JS shim.
    pub js_only: bool,
    /// If `true`, bypass the compilation cache even if `cache_dir` is set.
    pub no_cache: bool,
}

/// Result of a successful compilation.
#[derive(Debug)]
pub struct CompileResult {
    /// The Astrid-compatible plugin ID (lowercase, hyphens).
    pub astrid_id: String,
    /// The detected runtime tier.
    pub tier: PluginTier,
    /// The parsed `OpenClaw` manifest.
    pub manifest: OpenClawManifest,
    /// Whether the result was served from cache (Tier 1 only).
    pub cached: bool,
}

/// Run the full `OpenClaw` → Astrid compilation pipeline.
///
/// Detects the plugin tier, checks the compilation cache, transpiles,
/// shims, compiles to WASM (Tier 1) or writes the Node.js bridge (Tier 2),
/// and generates `Capsule.toml`.
///
/// # Errors
///
/// Returns [`BridgeError`] if any stage fails.
#[must_use = "compilation result contains the plugin ID and tier"]
pub fn compile_plugin(opts: &CompileOptions<'_>) -> BridgeResult<CompileResult> {
    // 1. Parse manifest and convert ID
    let oc_manifest = manifest::parse_manifest(opts.plugin_dir)?;
    let astrid_id = manifest::convert_id(&oc_manifest.id)?;

    // 2. Validate config against schema (skip required-field check at build time —
    //    config values are a runtime/install concern, not a compilation constraint)
    validate_config(opts.config, &oc_manifest.config_schema, false)?;

    // 3. Detect tier
    let plugin_tier = tier::detect_tier(opts.plugin_dir, Some(&oc_manifest));

    // 4. Ensure output directory exists
    std::fs::create_dir_all(opts.output_dir)?;

    match plugin_tier {
        PluginTier::Wasm => compile_tier1(opts, &oc_manifest, &astrid_id),
        PluginTier::Node => compile_tier2(opts, &oc_manifest, &astrid_id),
    }
}

/// Tier 1: Transpile → Shim → WASM compilation (with cache).
fn compile_tier1(
    opts: &CompileOptions<'_>,
    oc_manifest: &OpenClawManifest,
    astrid_id: &str,
) -> BridgeResult<CompileResult> {
    // Resolve and read entry point
    let entry_point_rel = manifest::resolve_entry_point(opts.plugin_dir)?;
    let entry_point = opts.plugin_dir.join(&entry_point_rel);
    if !entry_point.exists() {
        return Err(BridgeError::EntryPointNotFound(entry_point));
    }

    let raw_source = std::fs::read_to_string(&entry_point)?;

    // Transpile TS→JS
    let js_code = transpiler::transpile(&raw_source, &entry_point_rel)?;

    // Generate shim with plugin identity from manifest
    let identity = shim::PluginIdentity {
        id: &oc_manifest.id,
        name: oc_manifest.name.as_deref(),
        version: oc_manifest.version.as_deref(),
        description: oc_manifest.description.as_deref(),
    };
    let shim_code = shim::generate(&js_code, opts.config, &identity);

    // Write shim for debugging
    let shim_path = opts.output_dir.join("shim.js");
    std::fs::write(&shim_path, &shim_code)?;

    if opts.js_only {
        return Ok(CompileResult {
            astrid_id: astrid_id.to_string(),
            tier: PluginTier::Wasm,
            manifest: oc_manifest.clone(),
            cached: false,
        });
    }

    // Check cache
    let cache = build_cache(opts);
    let source_hash = blake3::hash(shim_code.as_bytes()).to_hex().to_string();

    if let Some(ref cache) = cache
        && let Some(hit) = cache.lookup(&source_hash, crate::VERSION)
    {
        // Write cached artifacts to output
        let wasm_path = opts.output_dir.join("plugin.wasm");
        std::fs::write(&wasm_path, &hit.wasm)?;

        let manifest_path = opts.output_dir.join("Capsule.toml");
        std::fs::write(&manifest_path, &hit.manifest)?;

        return Ok(CompileResult {
            astrid_id: astrid_id.to_string(),
            tier: PluginTier::Wasm,
            manifest: oc_manifest.clone(),
            cached: true,
        });
    }

    // Compile to WASM
    let wasm_path = opts.output_dir.join("plugin.wasm");
    compiler::compile(&shim_code, &wasm_path)?;

    // Generate Capsule.toml
    output::generate_manifest(
        astrid_id,
        oc_manifest,
        &wasm_path,
        opts.config,
        opts.output_dir,
    )?;

    // Store in cache (best-effort — never fail the pipeline on cache errors)
    if let Some(ref cache) = cache
        && let Ok(wasm) = std::fs::read(&wasm_path)
        && let Ok(manifest_content) = std::fs::read_to_string(opts.output_dir.join("Capsule.toml"))
        && let Err(e) = cache.store(&source_hash, crate::VERSION, &wasm, &manifest_content)
    {
        eprintln!("warning: failed to cache compilation result: {e}");
    }

    Ok(CompileResult {
        astrid_id: astrid_id.to_string(),
        tier: PluginTier::Wasm,
        manifest: oc_manifest.clone(),
        cached: false,
    })
}

/// Tier 2: Write Node.js MCP bridge + generate MCP-backed `Capsule.toml`.
fn compile_tier2(
    opts: &CompileOptions<'_>,
    oc_manifest: &OpenClawManifest,
    astrid_id: &str,
) -> BridgeResult<CompileResult> {
    // Resolve entry point
    let entry_point_rel = manifest::resolve_entry_point(opts.plugin_dir)?;
    let entry_point = opts.plugin_dir.join(&entry_point_rel);
    if !entry_point.exists() {
        return Err(BridgeError::EntryPointNotFound(entry_point));
    }

    // Guard: output_dir must not be inside plugin_dir (would cause infinite copy)
    if opts.output_dir.starts_with(opts.plugin_dir) {
        return Err(BridgeError::Manifest(
            "output_dir must not be inside plugin_dir".into(),
        ));
    }

    // Copy plugin source into output dir root (preserving directory structure)
    copy_plugin_source(opts.plugin_dir, opts.output_dir, 0)?;

    // Write the MCP bridge script
    node_bridge::write_bridge_script(opts.output_dir)?;

    // Generate Tier 2 Capsule.toml (MCP server instead of WASM component)
    generate_tier2_manifest(astrid_id, oc_manifest, &entry_point_rel, opts.output_dir)?;

    Ok(CompileResult {
        astrid_id: astrid_id.to_string(),
        tier: PluginTier::Node,
        manifest: oc_manifest.clone(),
        cached: false,
    })
}

/// Serializable Tier 2 `Capsule.toml` manifest.
#[derive(Debug, serde::Serialize)]
struct Tier2Manifest {
    package: Tier2Package,
    mcp_server: Vec<Tier2McpServer>,
    capabilities: Tier2Capabilities,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, Tier2EnvDef>,
}

#[derive(Debug, serde::Serialize)]
struct Tier2Package {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct Tier2McpServer {
    id: String,
    command: String,
    args: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct Tier2Capabilities {
    host_process: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct Tier2EnvDef {
    #[serde(rename = "type")]
    env_type: String,
    request: String,
}

/// Generate a `Capsule.toml` for Tier 2 plugins using `[[mcp_server]]`.
fn generate_tier2_manifest(
    astrid_id: &str,
    oc_manifest: &OpenClawManifest,
    entry_point_rel: &str,
    output_dir: &Path,
) -> BridgeResult<()> {
    // Validate schema property keys before using them as TOML keys
    let mut env = HashMap::new();
    if let Some(obj) = oc_manifest.config_schema.as_object()
        && let Some(props) = obj.get("properties").and_then(|p| p.as_object())
    {
        for (key, _val) in props {
            manifest::validate_schema_key(key)?;
            let env_type = if manifest::is_secret_key(key) {
                "secret"
            } else {
                "string"
            };
            env.insert(
                key.clone(),
                Tier2EnvDef {
                    env_type: env_type.to_string(),
                    request: format!("Please enter value for {key}"),
                },
            );
        }
    }

    let manifest = Tier2Manifest {
        package: Tier2Package {
            name: astrid_id.to_string(),
            version: oc_manifest.display_version().to_string(),
            description: oc_manifest.description.clone(),
        },
        mcp_server: vec![Tier2McpServer {
            id: astrid_id.to_string(),
            command: "node".to_string(),
            args: vec![
                "astrid_bridge.mjs".to_string(),
                "--entry".to_string(),
                entry_point_rel.to_string(),
                "--plugin-id".to_string(),
                astrid_id.to_string(),
            ],
        }],
        capabilities: Tier2Capabilities {
            host_process: vec!["node".to_string()],
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

/// Maximum nesting depth for plugin source tree traversal.
const MAX_COPY_DEPTH: usize = 64;

/// Copy plugin source files, skipping `node_modules`, `.git`, etc.
fn copy_plugin_source(src: &Path, dst: &Path, depth: usize) -> BridgeResult<()> {
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

/// Validate config values against the plugin's `configSchema`.
///
/// Used at both build time and activation time:
/// - **Build time** (`check_required: false`): validates that all provided keys
///   are declared in the schema, but does not enforce required fields (config
///   values are a runtime/install concern, not a compilation constraint).
/// - **Activation time** (`check_required: true`): additionally verifies that
///   all `required` properties are present in the config.
///
/// # Errors
///
/// Returns [`BridgeError::ConfigValidation`] if unknown keys are present or
/// required keys are missing (when `check_required` is `true`).
pub fn validate_config<S: std::hash::BuildHasher>(
    config: &HashMap<String, serde_json::Value, S>,
    schema: &serde_json::Value,
    check_required: bool,
) -> BridgeResult<()> {
    let Some(schema_obj) = schema.as_object() else {
        return Ok(());
    };

    let properties = schema_obj.get("properties").and_then(|p| p.as_object());

    // Check that all provided keys are declared in the schema
    if let Some(props) = properties {
        for key in config.keys() {
            if !props.contains_key(key) {
                return Err(BridgeError::ConfigValidation(format!(
                    "unknown config key '{key}' — not declared in configSchema.properties"
                )));
            }
        }
    }

    // Check that all required properties are present (only at install/activation time)
    if check_required && let Some(required) = schema_obj.get("required").and_then(|r| r.as_array())
    {
        let missing: Vec<&str> = required
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|key| !config.contains_key(*key))
            .collect();

        if !missing.is_empty() {
            return Err(BridgeError::ConfigValidation(format!(
                "missing required config keys: {}",
                missing.join(", ")
            )));
        }
    }

    Ok(())
}

/// Build a `CompilationCache` from options, if caching is enabled.
fn build_cache(opts: &CompileOptions<'_>) -> Option<CompilationCache> {
    if opts.no_cache {
        return None;
    }
    let cache_dir = opts.cache_dir?;
    Some(CompilationCache::new(
        cache_dir.to_path_buf(),
        compiler::kernel_hash(),
    ))
}

/// Run garbage collection on the compilation cache.
///
/// # Errors
///
/// Returns [`BridgeError::Cache`] if the cache directory cannot be read.
pub fn cache_gc(
    cache_dir: &Path,
    max_age_days: u64,
    max_size_bytes: u64,
) -> BridgeResult<crate::cache::GcStats> {
    let cache = CompilationCache::new(cache_dir.to_path_buf(), compiler::kernel_hash());
    cache.gc(max_age_days, max_size_bytes)
}

/// Resolve the default cache directory (`~/.astrid/cache/openclaw/`).
///
/// Returns `None` if the home directory cannot be determined.
#[must_use]
pub fn default_cache_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".astrid/cache/openclaw"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal `OpenClaw` plugin directory for testing.
    fn create_test_plugin(dir: &Path, source: &str) {
        let manifest = r#"{"id": "test-plugin", "configSchema": {}}"#;
        std::fs::write(dir.join("openclaw.plugin.json"), manifest).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/index.js"), source).unwrap();
    }

    /// Create a plugin that will be detected as Tier 2 (Node.js).
    fn create_tier2_plugin(dir: &Path) {
        let manifest = r#"{"id": "tier2-plugin", "configSchema": {}}"#;
        std::fs::write(dir.join("openclaw.plugin.json"), manifest).unwrap();
        // Having npm dependencies triggers Tier 2
        let pkg = r#"{"name": "tier2", "dependencies": {"axios": "^1.0.0"}}"#;
        std::fs::write(dir.join("package.json"), pkg).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/index.js"),
            "const axios = require('axios');\nmodule.exports = {};",
        )
        .unwrap();
    }

    fn simple_source() -> &'static str {
        "module.exports.activate = function(ctx) {};"
    }

    fn default_opts<'a>(
        plugin_dir: &'a Path,
        output_dir: &'a Path,
        config: &'a HashMap<String, serde_json::Value>,
    ) -> CompileOptions<'a> {
        CompileOptions {
            plugin_dir,
            output_dir,
            config,
            cache_dir: None,
            js_only: false,
            no_cache: true,
        }
    }

    #[test]
    fn tier_detection_wasm_for_simple_plugin() {
        let dir = tempfile::tempdir().unwrap();
        create_test_plugin(dir.path(), simple_source());

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let mut opts = default_opts(dir.path(), output.path(), &config);
        opts.js_only = true;

        let result = compile_plugin(&opts).unwrap();
        assert_eq!(result.tier, PluginTier::Wasm);
        assert_eq!(result.astrid_id, "test-plugin");
        assert!(!result.cached);
    }

    #[test]
    fn tier_detection_node_for_npm_deps() {
        let dir = tempfile::tempdir().unwrap();
        create_tier2_plugin(dir.path());

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let opts = default_opts(dir.path(), output.path(), &config);

        let result = compile_plugin(&opts).unwrap();
        assert_eq!(result.tier, PluginTier::Node);
        assert_eq!(result.astrid_id, "tier2-plugin");
    }

    #[test]
    fn tier2_generates_mcp_manifest() {
        let dir = tempfile::tempdir().unwrap();
        create_tier2_plugin(dir.path());

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let opts = default_opts(dir.path(), output.path(), &config);

        compile_plugin(&opts).unwrap();

        let capsule_toml = std::fs::read_to_string(output.path().join("Capsule.toml")).unwrap();
        assert!(
            capsule_toml.contains("[[mcp_server]]"),
            "Tier 2 should use mcp_server, got: {capsule_toml}"
        );
        assert!(
            capsule_toml.contains("command = \"node\""),
            "Tier 2 should use node"
        );
        assert!(
            capsule_toml.contains("host_process = [\"node\"]"),
            "Tier 2 should declare host_process"
        );
        assert!(
            !capsule_toml.contains("entrypoint"),
            "Tier 2 should not have WASM entrypoint"
        );
    }

    #[test]
    fn tier2_copies_source_and_bridge() {
        let dir = tempfile::tempdir().unwrap();
        create_tier2_plugin(dir.path());

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let opts = default_opts(dir.path(), output.path(), &config);

        compile_plugin(&opts).unwrap();

        assert!(
            output.path().join("astrid_bridge.mjs").exists(),
            "Bridge script should be written"
        );
        assert!(
            output.path().join("src/index.js").exists(),
            "Plugin source should be copied under src/"
        );
    }

    #[test]
    fn tier2_manifest_includes_env_with_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{
            "id": "env-plugin",
            "configSchema": {
                "type": "object",
                "properties": {
                    "apiKey": {"type": "string"},
                    "baseUrl": {"type": "string"}
                }
            }
        }"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        let pkg = r#"{"name": "env", "dependencies": {"got": "^1.0"}}"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/index.js"), "module.exports = {};").unwrap();

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let opts = default_opts(dir.path(), output.path(), &config);

        compile_plugin(&opts).unwrap();

        let capsule_toml = std::fs::read_to_string(output.path().join("Capsule.toml")).unwrap();
        assert!(
            capsule_toml.contains("type = \"secret\""),
            "apiKey should be detected as secret"
        );
        assert!(
            capsule_toml.contains("type = \"string\""),
            "baseUrl should be plain string"
        );
    }

    #[test]
    fn tier1_js_only_skips_wasm() {
        let dir = tempfile::tempdir().unwrap();
        create_test_plugin(dir.path(), simple_source());

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let mut opts = default_opts(dir.path(), output.path(), &config);
        opts.js_only = true;

        let result = compile_plugin(&opts).unwrap();
        assert_eq!(result.tier, PluginTier::Wasm);
        assert!(output.path().join("shim.js").exists(), "Shim should exist");
        assert!(
            !output.path().join("plugin.wasm").exists(),
            "WASM should not exist in js_only mode"
        );
    }

    #[test]
    fn compile_plugin_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        create_test_plugin(dir.path(), simple_source());

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let mut opts = default_opts(dir.path(), output.path(), &config);
        opts.js_only = true;

        let r1 = compile_plugin(&opts).unwrap();
        let r2 = compile_plugin(&opts).unwrap();
        assert_eq!(r1.astrid_id, r2.astrid_id);
        assert_eq!(r1.tier, r2.tier);
    }

    #[test]
    fn compile_plugin_errors_on_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        // No openclaw.plugin.json

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let opts = default_opts(dir.path(), output.path(), &config);

        let err = compile_plugin(&opts).unwrap_err();
        assert!(
            matches!(err, BridgeError::Manifest(_)),
            "expected Manifest error, got: {err}"
        );
    }

    #[test]
    fn compile_plugin_errors_on_missing_entry_point() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"id": "no-entry", "configSchema": {}}"#;
        std::fs::write(dir.path().join("openclaw.plugin.json"), manifest).unwrap();
        // No src/index.js — entry point resolution will fail

        let output = tempfile::tempdir().unwrap();
        let config = HashMap::new();
        let opts = default_opts(dir.path(), output.path(), &config);

        let err = compile_plugin(&opts).unwrap_err();
        assert!(
            matches!(err, BridgeError::EntryPointNotFound(_)),
            "expected EntryPointNotFound error, got: {err}"
        );
    }

    #[test]
    fn default_cache_dir_is_some() {
        let dir = default_cache_dir();
        assert!(
            dir.is_some(),
            "default_cache_dir should resolve on systems with a home directory"
        );
        let path = dir.unwrap();
        assert!(
            path.ends_with("openclaw"),
            "cache dir should end with 'openclaw', got: {path:?}"
        );
    }

    #[test]
    fn cache_gc_on_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let stats = cache_gc(dir.path(), 30, 500_000_000).unwrap();
        assert_eq!(stats.entries_removed, 0);
    }

    #[test]
    fn copy_plugin_source_skips_node_modules() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("node_modules/foo")).unwrap();
        std::fs::write(src.path().join("node_modules/foo/bar.js"), "x").unwrap();
        std::fs::write(src.path().join("index.js"), "y").unwrap();

        let dst = tempfile::tempdir().unwrap();
        copy_plugin_source(src.path(), dst.path(), 0).unwrap();

        assert!(dst.path().join("index.js").exists());
        assert!(
            !dst.path().join("node_modules").exists(),
            "node_modules should be skipped"
        );
    }

    #[test]
    fn copy_plugin_source_skips_git_dir() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join(".git/objects")).unwrap();
        std::fs::write(src.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(src.path().join("index.js"), "y").unwrap();

        let dst = tempfile::tempdir().unwrap();
        copy_plugin_source(src.path(), dst.path(), 0).unwrap();

        assert!(dst.path().join("index.js").exists());
        assert!(!dst.path().join(".git").exists(), ".git should be skipped");
    }

    // ── Config validation tests ──

    #[test]
    fn validate_config_accepts_valid_keys() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "apiKey": {"type": "string"},
                "baseUrl": {"type": "string"}
            }
        });
        let mut config = HashMap::new();
        config.insert("apiKey".into(), serde_json::json!("sk-123"));
        assert!(validate_config(&config, &schema, true).is_ok());
    }

    #[test]
    fn validate_config_rejects_unknown_key() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "apiKey": {"type": "string"}
            }
        });
        let mut config = HashMap::new();
        config.insert("bogusKey".into(), serde_json::json!("val"));
        let err = validate_config(&config, &schema, true).unwrap_err();
        assert!(
            matches!(err, BridgeError::ConfigValidation(ref msg) if msg.contains("bogusKey")),
            "expected ConfigValidation with bogusKey, got: {err}"
        );
    }

    #[test]
    fn validate_config_rejects_missing_required() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "apiKey": {"type": "string"},
                "model": {"type": "string"}
            },
            "required": ["apiKey", "model"]
        });
        let mut config = HashMap::new();
        config.insert("apiKey".into(), serde_json::json!("sk-123"));
        let err = validate_config(&config, &schema, true).unwrap_err();
        assert!(
            matches!(err, BridgeError::ConfigValidation(ref msg) if msg.contains("model")),
            "expected ConfigValidation mentioning model, got: {err}"
        );
    }

    #[test]
    fn validate_config_empty_schema_accepts_anything() {
        let schema = serde_json::json!({});
        let mut config = HashMap::new();
        config.insert("anything".into(), serde_json::json!("val"));
        assert!(validate_config(&config, &schema, true).is_ok());
    }

    #[test]
    fn validate_config_empty_config_with_required_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "apiKey": {"type": "string"}
            },
            "required": ["apiKey"]
        });
        let config = HashMap::new();
        // With check_required=true, this should fail
        let err = validate_config(&config, &schema, true).unwrap_err();
        assert!(
            matches!(err, BridgeError::ConfigValidation(ref msg) if msg.contains("apiKey")),
            "expected ConfigValidation with apiKey, got: {err}"
        );
        // With check_required=false (build-time), this should pass
        assert!(validate_config(&config, &schema, false).is_ok());
    }

    #[test]
    fn validate_config_non_object_schema_accepts_anything() {
        let schema = serde_json::json!(true);
        let config = HashMap::new();
        assert!(validate_config(&config, &schema, true).is_ok());
    }
}
