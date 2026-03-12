//! Capsule management commands - install capsules securely.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use astrid_capsule::discovery::load_manifest;
use astrid_core::dirs::AstridHome;

/// Update one or all installed capsules by re-installing from their original source.
///
/// If `target` is `Some`, update only that capsule. If `None`, update all capsules
/// that have a recorded source in `meta.json`.
///
/// # TODO
/// - When `target` is `None` (bare `astrid capsule update`), this should check all
///   installed capsules for newer versions from their original repo/registry source
///   before re-installing, rather than blindly re-fetching everything.
/// - Add a registry manifest (like brew formulas) that pins version + Blake3 hash
///   per capsule. `update` should fetch the manifest, compare versions against
///   `meta.json`, only download if newer, and verify Blake3 hash before installing.
///   Trust chain: registry manifest (signed) -> pinned URL + Blake3 -> verified binary.
pub(crate) fn update_capsule(target: Option<&str>, workspace: bool) -> anyhow::Result<()> {
    let home = AstridHome::resolve()?;

    if let Some(name) = target {
        // Update a single capsule
        let target_dir = resolve_target_dir(&home, name, workspace)?;
        if !target_dir.exists() {
            bail!("Capsule '{name}' is not installed.");
        }

        let meta = read_meta(&target_dir)
            .ok_or_else(|| anyhow::anyhow!("Capsule '{name}' has no meta.json - cannot determine original source. Re-install it manually."))?;

        let source = meta.source.ok_or_else(|| {
            anyhow::anyhow!(
                "Capsule '{name}' was installed before source tracking was added. Re-install it manually to record the source."
            )
        })?;

        eprintln!("Updating {name} from {source}...");
        install_capsule(&source, workspace)
    } else {
        // TODO: Check all installed capsules for updates from their repo/registry
        // source. For now, re-install everything that has a recorded source.
        let capsules_dir = home.capsules_dir();
        if !capsules_dir.exists() {
            eprintln!("No capsules installed.");
            return Ok(());
        }

        let mut updated = 0u32;
        let mut skipped = 0u32;
        for entry in std::fs::read_dir(&capsules_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let meta = read_meta(&entry.path());
            if let Some(meta) = meta
                && let Some(ref source) = meta.source
            {
                eprintln!("Updating {name} from {source}...");
                if let Err(e) = install_capsule(source, workspace) {
                    eprintln!("Failed to update {name}: {e}");
                } else {
                    updated = updated.saturating_add(1);
                }
            } else {
                eprintln!("Skipping {name} (no source recorded).");
                skipped = skipped.saturating_add(1);
            }
        }

        eprintln!("Updated {updated} capsule(s), skipped {skipped}.");
        Ok(())
    }
}

pub(crate) fn install_capsule(source: &str, workspace: bool) -> anyhow::Result<()> {
    let home = AstridHome::resolve()?;

    // 1. Explicit Local Path
    if source.starts_with('.') || source.starts_with('/') {
        return install_from_local(source, workspace, &home, Some(source));
    }

    // 2. OpenClaw Explicit Prefix
    if let Some(rest) = source.strip_prefix("openclaw:") {
        // If it uses the github namespace alias after the prefix
        if let Some(repo) = rest.strip_prefix('@') {
            let url = format!("https://github.com/{repo}");
            return install_from_github(&url, workspace, &home, true, Some(source));
        }
        return install_from_openclaw(rest, workspace, &home, Some(source));
    }

    // 3. Native Namespace Alias (@org/repo) -> GitHub
    if let Some(repo) = source.strip_prefix('@') {
        let url = format!("https://github.com/{repo}");
        return install_from_github(&url, workspace, &home, false, Some(source));
    }

    // 4. Raw GitHub URL
    if source.starts_with("github.com/") || source.starts_with("https://github.com/") {
        return install_from_github(source, workspace, &home, false, Some(source));
    }

    // 5. Fallback: Assume it's a local folder matching the given name
    install_from_local(source, workspace, &home, Some(source))
}

pub(crate) fn install_from_github(
    url: &str,
    workspace: bool,
    home: &AstridHome,
    _is_openclaw: bool,
    original_source: Option<&str>,
) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("astrid-cli")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url_trimmed = url.trim_end_matches('/');
    let mut parts: Vec<&str> = url_trimmed.split('/').collect();
    if parts.len() < 2 {
        bail!("Invalid GitHub URL format. Expected github.com/org/repo or @org/repo");
    }
    let repo = parts.pop().context("Failed to get repo name from URL")?;
    let org = parts.pop().context("Failed to get org name from URL")?;

    let api_url = format!("https://api.github.com/repos/{org}/{repo}/releases/latest");

    let res = client.get(&api_url).send();

    if let Ok(response) = res
        && response.status().is_success()
        && let Ok(json) = response.json::<serde_json::Value>()
        && let Some(assets) = json.get("assets").and_then(serde_json::Value::as_array)
    {
        for asset in assets {
            if let Some(name) = asset.get("name").and_then(serde_json::Value::as_str)
                && name.ends_with(".capsule")
                && let Some(download_url) = asset
                    .get("browser_download_url")
                    .and_then(serde_json::Value::as_str)
            {
                let tmp_dir = tempfile::tempdir()?;
                let sanitized_name = Path::new(name).file_name().unwrap_or_default();
                let download_path = tmp_dir.path().join(sanitized_name);
                let mut file = std::fs::File::create(&download_path)?;

                let download_res = client.get(download_url).send()?;

                // Enforce a strict 50MB download limit to prevent DoS attacks
                let mut limited_stream = download_res.take(50 * 1024 * 1024);
                std::io::copy(&mut limited_stream, &mut file)?;

                return unpack_and_install(&download_path, workspace, home, original_source);
            }
        }
    }

    let tmp_dir = tempfile::tempdir().context("failed to create temp dir for cloning")?;
    let clone_dir = tmp_dir.path().join(repo);

    let status = std::process::Command::new("git")
        .args(["clone", "--depth", "1", url, &clone_dir.to_string_lossy()])
        .status()
        .context("Failed to spawn git clone")?;

    if !status.success() {
        bail!("Failed to clone repository from GitHub.");
    }

    let output_dir = tmp_dir.path().join("dist");
    std::fs::create_dir_all(&output_dir)?;

    crate::commands::build::run_build(
        Some(clone_dir.to_str().context("Invalid clone dir path")?),
        Some(output_dir.to_str().context("Invalid output dir path")?),
        None,
        None,
    )?;

    // Find the .capsule file
    for entry in std::fs::read_dir(&output_dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) == Some("capsule") {
            return unpack_and_install(&entry.path(), workspace, home, original_source);
        }
    }

    bail!("Universal Migrator failed to produce a .capsule archive.");
}

pub(crate) fn install_from_openclaw(
    source: &str,
    workspace: bool,
    home: &AstridHome,
    original_source: Option<&str>,
) -> anyhow::Result<()> {
    let capsule_name = source.strip_prefix("openclaw:").unwrap_or(source);

    // Step 1: Mock Registry Fetch
    // In a real implementation, this would hit https://registry.openclaw.io
    // For now, we assume the user might have a local directory with that name for testing,
    // or we just bail if it doesn't exist locally as a fallback.
    let source_path = Path::new(capsule_name);
    if !source_path.exists() {
        bail!(
            "OpenClaw registry fetch not yet implemented. Please provide a local path to the OpenClaw capsule directory."
        );
    }

    transpile_and_install(source_path, workspace, home, original_source)
}

pub(crate) fn transpile_and_install(
    source_path: &Path,
    workspace: bool,
    home: &AstridHome,
    original_source: Option<&str>,
) -> anyhow::Result<()> {
    let tmp_dir = tempfile::tempdir().context("failed to create temp dir for transpilation")?;
    let output_dir = tmp_dir.path();

    let cache_dir = astrid_openclaw::pipeline::default_cache_dir();

    // Config is empty at install time — required-field validation happens at
    // capsule activation when config values are actually available.
    // See `pipeline::validate_config(check_required: true)`.
    let opts = astrid_openclaw::pipeline::CompileOptions {
        plugin_dir: source_path,
        output_dir,
        config: &HashMap::new(),
        cache_dir: cache_dir.as_deref(),
        js_only: false,
        no_cache: false,
    };

    let result = astrid_openclaw::pipeline::compile_plugin(&opts)
        .map_err(|e| anyhow::anyhow!("OpenClaw compilation failed: {e}"))?;

    eprintln!(
        "Compiled {} v{} (tier: {}, cached: {})",
        result.manifest.display_name(),
        result.manifest.display_version(),
        result.tier,
        result.cached,
    );

    // Proceed with standard installation from the temp directory
    install_from_local_path_inner(output_dir, workspace, home, original_source)
}

pub(crate) fn install_from_local(
    source: &str,
    workspace: bool,
    home: &AstridHome,
    original_source: Option<&str>,
) -> anyhow::Result<()> {
    let source_path = Path::new(source);
    if !source_path.exists() {
        bail!("Source path does not exist: {source}");
    }

    // Auto-detect OpenClaw
    if source_path.join("openclaw.plugin.json").exists()
        && !source_path.join("Capsule.toml").exists()
    {
        return transpile_and_install(source_path, workspace, home, original_source);
    }

    // Unpack .capsule archive if it is a file
    if source_path.is_file() && source.ends_with(".capsule") {
        return unpack_and_install(source_path, workspace, home, original_source);
    }

    // Auto-build Rust capsules if we point at a source directory with a Cargo.toml
    if source_path.is_dir() && source_path.join("Cargo.toml").exists() {
        let tmp_dir = tempfile::tempdir().context("failed to create temp dir for building")?;
        let output_dir = tmp_dir.path().join("dist");

        crate::commands::build::run_build(
            Some(source),
            Some(output_dir.to_str().context("Invalid output dir path")?),
            Some("rust"),
            None,
        )?;

        // Find the newly built .capsule file in the output directory
        for entry in std::fs::read_dir(&output_dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("capsule") {
                return unpack_and_install(&entry.path(), workspace, home, original_source);
            }
        }
        bail!("Failed to auto-build capsule from Cargo project.");
    }

    install_from_local_path_inner(source_path, workspace, home, original_source)
}

fn unpack_and_install(
    archive_path: &Path,
    workspace: bool,
    home: &AstridHome,
    original_source: Option<&str>,
) -> anyhow::Result<()> {
    let tmp_dir = tempfile::tempdir().context("failed to create temp dir for unpacking")?;
    let unpack_dir = tmp_dir.path();

    let tar_gz = std::fs::File::open(archive_path)
        .with_context(|| format!("Failed to open archive: {}", archive_path.display()))?;

    let tar = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(tar);

    // Safely unpack the archive, verifying paths to prevent traversal and symlink attacks
    for entry in archive
        .entries()
        .context("Failed to read archive entries")?
    {
        let mut entry = entry.context("Failed to read archive entry")?;
        let entry_path = entry.path().context("Invalid path in archive")?;

        // Prevent absolute paths or path traversal (..)
        if entry_path.is_absolute()
            || entry_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            bail!(
                "Malicious archive detected: invalid path '{}'",
                entry_path.display()
            );
        }

        let out_path = unpack_dir.join(&entry_path);

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Deny symlinks to prevent arbitrary file writes
        if entry.header().entry_type().is_symlink() || entry.header().entry_type().is_hard_link() {
            bail!(
                "Malicious archive detected: symlinks are not allowed ('{}')",
                entry_path.display()
            );
        }

        entry
            .unpack(&out_path)
            .with_context(|| format!("Failed to unpack file: {}", out_path.display()))?;
    }

    install_from_local_path_inner(unpack_dir, workspace, home, original_source)
}

/// Capsule installation metadata, persisted as `meta.json` alongside `Capsule.toml`.
#[derive(Debug, Serialize, Deserialize)]
struct CapsuleMeta {
    /// The currently installed version.
    version: String,
    /// When the capsule was first installed.
    installed_at: String,
    /// When the capsule was last updated.
    updated_at: String,
    /// The original install source (local path, GitHub URL, openclaw: prefix, etc.).
    /// Used by `astrid capsule update` to re-fetch from the same source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

/// Read existing `meta.json` from a capsule's install directory (if present).
fn read_meta(target_dir: &Path) -> Option<CapsuleMeta> {
    let meta_path = target_dir.join("meta.json");
    let data = std::fs::read_to_string(&meta_path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Write `meta.json` to the capsule's install directory.
fn write_meta(target_dir: &Path, meta: &CapsuleMeta) -> anyhow::Result<()> {
    let meta_path = target_dir.join("meta.json");
    let json = serde_json::to_string_pretty(meta).context("failed to serialize meta.json")?;
    std::fs::write(&meta_path, json)
        .with_context(|| format!("failed to write {}", meta_path.display()))
}

pub(crate) fn install_from_local_path(
    source_path: &Path,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    install_from_local_path_inner(source_path, workspace, home, None)
}

pub(crate) fn install_from_local_path_inner(
    source_path: &Path,
    workspace: bool,
    home: &AstridHome,
    original_source: Option<&str>,
) -> anyhow::Result<()> {
    let manifest_path = source_path.join("Capsule.toml");
    if !manifest_path.exists() {
        bail!("No Capsule.toml found in {}", source_path.display());
    }

    let manifest = load_manifest(&manifest_path).context("failed to load Capsule manifest")?;
    let id = manifest.package.name.clone();
    let new_version = manifest.package.version.clone();

    // Resolve Target Directory
    let target_dir = resolve_target_dir(home, &id, workspace)?;
    let parent = target_dir.parent().context("target dir has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    // Detect install vs upgrade by checking existing meta.json
    let existing_meta = read_meta(&target_dir);
    let (phase, previous_version) = if let Some(ref meta) = existing_meta {
        (
            astrid_capsule::engine::wasm::host_state::LifecyclePhase::Upgrade,
            Some(meta.version.clone()),
        )
    } else {
        (
            astrid_capsule::engine::wasm::host_state::LifecyclePhase::Install,
            None,
        )
    };

    // Backup existing target if present (for rollback on lifecycle failure).
    let backup_dir = if target_dir.exists() {
        let backup = target_dir.with_extension("bak");
        if backup.exists() {
            std::fs::remove_dir_all(&backup)?;
        }
        std::fs::rename(&target_dir, &backup)?;
        Some(backup)
    } else {
        None
    };

    // Copy capsule files
    if let Err(e) = copy_capsule_dir(source_path, &target_dir) {
        // Restore backup on copy failure
        if let Some(ref backup) = backup_dir {
            let _ = std::fs::remove_dir_all(&target_dir);
            let _ = std::fs::rename(backup, &target_dir);
        }
        return Err(e);
    }

    // Run lifecycle hook if a WASM binary exists
    if let Err(e) = run_lifecycle_if_wasm(
        &target_dir,
        &manifest,
        &id,
        phase,
        previous_version.as_deref(),
    ) {
        eprintln!("Lifecycle hook failed: {e}");
        // Rollback: restore previous installation
        let _ = std::fs::remove_dir_all(&target_dir);
        if let Some(ref backup) = backup_dir {
            let _ = std::fs::rename(backup, &target_dir);
        }
        return Err(e);
    }

    // Write meta.json on success
    let now = chrono::Utc::now().to_rfc3339();
    let meta = CapsuleMeta {
        version: new_version,
        installed_at: existing_meta
            .as_ref()
            .map_or_else(|| now.clone(), |m| m.installed_at.clone()),
        updated_at: now,
        source: original_source
            .map(String::from)
            .or_else(|| existing_meta.and_then(|m| m.source)),
    };
    write_meta(&target_dir, &meta)?;

    // Clean up backup
    if let Some(ref backup) = backup_dir {
        let _ = std::fs::remove_dir_all(backup);
    }

    Ok(())
}

/// Run lifecycle hooks if the capsule contains a WASM binary.
///
/// Non-WASM capsules (OpenClaw/JS) are skipped silently.
fn run_lifecycle_if_wasm(
    target_dir: &Path,
    manifest: &astrid_capsule::manifest::CapsuleManifest,
    capsule_id: &str,
    phase: astrid_capsule::engine::wasm::host_state::LifecyclePhase,
    previous_version: Option<&str>,
) -> anyhow::Result<()> {
    // Find the WASM binary from the manifest's component definitions
    let Some(component) = manifest.components.first() else {
        return Ok(()); // No components - skip lifecycle
    };

    let wasm_path = if component.path.is_absolute() {
        component.path.clone()
    } else {
        target_dir.join(&component.path)
    };

    if !wasm_path.exists() || wasm_path.extension().and_then(|e| e.to_str()) != Some("wasm") {
        return Ok(()); // Not a WASM capsule - skip lifecycle
    }

    let wasm_bytes = std::fs::read(&wasm_path)
        .with_context(|| format!("failed to read WASM binary: {}", wasm_path.display()))?;

    // Build minimal infrastructure for lifecycle dispatch
    let kv_store = std::sync::Arc::new(astrid_storage::MemoryKvStore::new());
    let kv = astrid_storage::ScopedKvStore::new(kv_store, format!("plugin:{capsule_id}"))
        .context("failed to create scoped KV store")?;
    let event_bus = astrid_events::EventBus::with_capacity(128);

    // Create a temporary tokio runtime for lifecycle dispatch.
    // We need this because the host functions use block_on internally.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for lifecycle")?;

    // Spawn a CLI-inline elicit handler that prompts on stdin.
    // Runs as a tokio task so we can use the async EventReceiver::recv().
    let elicit_bus = event_bus.clone();
    let elicit_receiver = event_bus.subscribe_topic("astrid.lifecycle.elicit");
    let elicit_handle = rt.spawn(async move {
        cli_elicit_handler(elicit_receiver, elicit_bus).await;
    });

    let capsule_id_owned = astrid_capsule::capsule::CapsuleId::new(capsule_id.to_string())
        .map_err(|e| anyhow::anyhow!("invalid capsule ID: {e}"))?;
    let cfg = astrid_capsule::engine::wasm::LifecycleConfig {
        wasm_bytes,
        capsule_id: capsule_id_owned,
        workspace_root: target_dir.to_path_buf(),
        kv,
        event_bus: event_bus.clone(),
        config: std::collections::HashMap::new(),
    };

    let result = rt.block_on(async {
        tokio::task::block_in_place(|| {
            astrid_capsule::engine::wasm::run_lifecycle(cfg, phase, previous_version)
        })
    });

    // Signal the elicit handler to stop
    elicit_handle.abort();
    drop(event_bus);
    drop(rt);

    result.map_err(|e| anyhow::anyhow!("lifecycle dispatch failed: {e}"))
}

/// Prompt the user on stdin for a single elicit field (runs in a blocking thread).
///
/// Returns `(value, values)` where exactly one is `Some`.
async fn prompt_stdin_field(
    prompt: String,
    field_type: astrid_events::ipc::OnboardingFieldType,
    default: Option<String>,
) -> (Option<String>, Option<Vec<String>>) {
    use astrid_events::ipc::OnboardingFieldType;

    match field_type {
        OnboardingFieldType::Text => {
            let val = tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let hint = default
                    .as_ref()
                    .map(|d| format!(" [{d}]"))
                    .unwrap_or_default();
                print!("{prompt}{hint}: ");
                let _ = std::io::stdout().flush();
                let mut input = String::new();
                let _ = std::io::stdin().read_line(&mut input);
                let input = input.trim().to_string();
                if input.is_empty() {
                    default.unwrap_or_default()
                } else {
                    input
                }
            })
            .await
            .unwrap_or_default();
            (Some(val), None)
        },
        OnboardingFieldType::Secret => {
            let val = tokio::task::spawn_blocking(move || {
                use std::io::Write;
                print!("{prompt} (secret, input hidden): ");
                let _ = std::io::stdout().flush();
                let mut input = String::new();
                let _ = std::io::stdin().read_line(&mut input);
                input.trim().to_string()
            })
            .await
            .unwrap_or_default();
            (Some(val), None)
        },
        OnboardingFieldType::Enum(options) => {
            let val = tokio::task::spawn_blocking(move || {
                use std::io::Write;
                println!("{prompt}:");
                for (i, opt) in options.iter().enumerate() {
                    println!("  {}: {opt}", i.saturating_add(1));
                }
                print!("Select [1-{}]: ", options.len());
                let _ = std::io::stdout().flush();
                let mut input = String::new();
                let _ = std::io::stdin().read_line(&mut input);
                let idx: usize = input.trim().parse().unwrap_or(0);
                if idx >= 1 && idx <= options.len() {
                    options[idx.saturating_sub(1)].clone()
                } else {
                    options.first().cloned().unwrap_or_default()
                }
            })
            .await
            .unwrap_or_default();
            (Some(val), None)
        },
        OnboardingFieldType::Array => {
            let items = tokio::task::spawn_blocking(move || {
                use std::io::Write;
                println!("{prompt} (enter values one per line, empty line to finish):");
                let mut items = Vec::new();
                loop {
                    print!("> ");
                    let _ = std::io::stdout().flush();
                    let mut input = String::new();
                    let _ = std::io::stdin().read_line(&mut input);
                    let input = input.trim().to_string();
                    if input.is_empty() {
                        break;
                    }
                    items.push(input);
                }
                items
            })
            .await
            .unwrap_or_default();
            (None, Some(items))
        },
    }
}

/// CLI-inline elicit handler for non-TUI installs.
///
/// Listens for `ElicitRequest` IPC messages and prompts on stdin,
/// then publishes `ElicitResponse` back to the event bus.
async fn cli_elicit_handler(
    mut receiver: astrid_events::EventReceiver,
    event_bus: astrid_events::EventBus,
) {
    use astrid_events::AstridEvent;
    use astrid_events::ipc::IpcPayload;

    loop {
        let Some(event) = receiver.recv().await else {
            return;
        };

        let AstridEvent::Ipc { message, .. } = &*event else {
            continue;
        };

        let IpcPayload::ElicitRequest {
            request_id,
            capsule_id,
            field,
        } = &message.payload
        else {
            continue;
        };

        let request_id = *request_id;
        let prompt = field.description.as_ref().map_or_else(
            || format!("[{capsule_id}] {}", field.key),
            |d| format!("[{capsule_id}] {d}"),
        );

        let (value, values) =
            prompt_stdin_field(prompt, field.field_type.clone(), field.default.clone()).await;

        let response_topic = format!("astrid.lifecycle.elicit.response.{request_id}");
        let response = IpcPayload::ElicitResponse {
            request_id,
            value,
            values,
        };
        let msg = astrid_events::ipc::IpcMessage::new(response_topic, response, uuid::Uuid::nil());
        event_bus.publish(AstridEvent::Ipc {
            message: msg,
            metadata: astrid_events::EventMetadata::default(),
        });
    }
}

/// Recursively copy a directory tree, dereferencing symlinks.
///
/// Symlinks are resolved to their target content (like `cp -rL`). This is
/// required because `npm install` creates symlinks in `node_modules/.bin/`
/// and the archiver also dereferences them via `follow_symlinks(true)`.
pub(crate) fn copy_capsule_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            let name = entry.file_name();
            if name == ".git" || name == "dist" || name == "target" {
                continue;
            }
            copy_capsule_dir(&src_path, &dst_path)?;
        } else if file_type.is_symlink() {
            // Dereference symlinks: resolve to the target's content and copy as
            // a regular file. This handles npm's node_modules/.bin/ symlinks.
            // fs::copy follows symlinks by default (reads the target, not the link).
            let metadata = std::fs::metadata(&src_path)
                .with_context(|| format!("symlink target not found for {}", src_path.display()))?;
            if metadata.is_dir() {
                // Symlink points to a directory - recurse into it
                copy_capsule_dir(&src_path, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)
                    .with_context(|| format!("failed to copy {}", src_path.display()))?;
            }
        } else {
            std::fs::copy(&src_path, &dst_path)
                .with_context(|| format!("failed to copy {}", src_path.display()))?;
        }
    }
    Ok(())
}

fn resolve_target_dir(
    home: &AstridHome,
    id: &str,
    workspace: bool,
) -> anyhow::Result<std::path::PathBuf> {
    if workspace {
        let root = std::env::current_dir().context("could not determine current directory")?;
        Ok(root.join(".astrid").join("capsules").join(id))
    } else {
        Ok(home.capsules_dir().join(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_preserves_node_modules() {
        // End-to-end: create a capsule directory with node_modules, install it
        // via install_from_local_path, and verify node_modules is preserved in
        // the installed directory.
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();

        // Minimal Capsule.toml
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"install-test\"\nversion = \"1.0.0\"\n\n\
             [[mcp_server]]\nid = \"install-test\"\ncommand = \"node\"\nargs = [\"bridge.mjs\"]\n",
        )
        .unwrap();

        // Bridge script
        std::fs::write(base.join("bridge.mjs"), "// bridge").unwrap();

        // Source
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(base.join("src/index.js"), "module.exports = {};").unwrap();

        // package.json + node_modules (simulating npm install output)
        std::fs::write(
            base.join("package.json"),
            r#"{"name": "install-test", "dependencies": {"got": "^1.0"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(base.join("node_modules/got")).unwrap();
        std::fs::write(
            base.join("node_modules/got/index.js"),
            "module.exports = {};",
        )
        .unwrap();

        // Install into a fake AstridHome
        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        install_from_local_path(base, false, &home).expect("install should succeed");

        // Verify installed directory preserves node_modules
        let installed = home.capsules_dir().join("install-test");
        assert!(
            installed.join("Capsule.toml").exists(),
            "installed capsule must have Capsule.toml"
        );
        assert!(
            installed.join("node_modules/got/index.js").exists(),
            "installed capsule must preserve node_modules"
        );
        assert!(
            installed.join("package.json").exists(),
            "installed capsule must preserve package.json"
        );
        assert!(
            installed.join("src/index.js").exists(),
            "installed capsule must preserve source"
        );
    }

    #[test]
    fn copy_capsule_dir_skips_git_and_build_artifacts() {
        let src_dir = tempfile::tempdir().unwrap();
        let base = src_dir.path();

        std::fs::write(base.join("index.js"), "// code").unwrap();
        std::fs::create_dir_all(base.join(".git/objects")).unwrap();
        std::fs::write(base.join(".git/objects/abc"), "blob").unwrap();
        std::fs::create_dir_all(base.join("dist")).unwrap();
        std::fs::write(base.join("dist/out.js"), "// built").unwrap();
        std::fs::create_dir_all(base.join("target")).unwrap();
        std::fs::write(base.join("target/debug"), "// rust").unwrap();
        std::fs::create_dir_all(base.join("node_modules/pkg")).unwrap();
        std::fs::write(base.join("node_modules/pkg/index.js"), "// dep").unwrap();

        let dst_dir = tempfile::tempdir().unwrap();
        copy_capsule_dir(base, dst_dir.path()).unwrap();

        assert!(dst_dir.path().join("index.js").exists());
        assert!(
            dst_dir.path().join("node_modules/pkg/index.js").exists(),
            "node_modules must be preserved"
        );
        assert!(
            !dst_dir.path().join(".git").exists(),
            ".git must be skipped"
        );
        assert!(
            !dst_dir.path().join("dist").exists(),
            "dist must be skipped"
        );
        assert!(
            !dst_dir.path().join("target").exists(),
            "target must be skipped"
        );
    }

    #[test]
    #[cfg_attr(windows, ignore = "symlinks require elevated privileges on Windows")]
    fn install_dereferences_node_modules_bin_symlinks() {
        // Simulates the realistic npm install output: node_modules/.bin/
        // contains relative symlinks to package executables. copy_capsule_dir
        // must dereference these into regular files instead of bailing.
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();

        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"symlink-test\"\nversion = \"1.0.0\"\n\n\
             [[mcp_server]]\nid = \"symlink-test\"\ncommand = \"node\"\nargs = [\"bridge.mjs\"]\n",
        )
        .unwrap();
        std::fs::write(base.join("bridge.mjs"), "// bridge").unwrap();

        // Create node_modules with a .bin/ symlink (like npm install produces)
        std::fs::create_dir_all(base.join("node_modules/somepkg")).unwrap();
        std::fs::write(
            base.join("node_modules/somepkg/cli.js"),
            "#!/usr/bin/env node\nconsole.log('works');",
        )
        .unwrap();
        std::fs::create_dir_all(base.join("node_modules/.bin")).unwrap();

        // Relative symlink — this is what npm actually creates
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            std::path::Path::new("../somepkg/cli.js"),
            base.join("node_modules/.bin/somepkg"),
        )
        .unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(
            std::path::Path::new("../somepkg/cli.js"),
            base.join("node_modules/.bin/somepkg"),
        )
        .unwrap();

        // Install must succeed (previously bailed on symlink)
        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        install_from_local_path(base, false, &home).expect("install must not bail on symlinks");

        // Verify the symlink was dereferenced into a regular file
        let installed = home.capsules_dir().join("symlink-test");
        let bin_file = installed.join("node_modules/.bin/somepkg");
        assert!(
            bin_file.exists(),
            ".bin/somepkg must exist as a regular file"
        );
        assert!(
            !bin_file.is_symlink(),
            ".bin/somepkg must be a regular file, not a symlink"
        );
        let content = std::fs::read_to_string(&bin_file).unwrap();
        assert!(
            content.contains("works"),
            "dereferenced file must have original content"
        );
    }

    #[test]
    fn meta_json_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let meta = CapsuleMeta {
            version: "1.2.3".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-03-12T00:00:00Z".into(),
            source: Some("@org/my-capsule".into()),
        };
        write_meta(dir.path(), &meta).unwrap();
        let loaded = read_meta(dir.path()).expect("meta should be readable");
        assert_eq!(loaded.version, "1.2.3");
        assert_eq!(loaded.installed_at, "2026-01-01T00:00:00Z");
        assert_eq!(loaded.updated_at, "2026-03-12T00:00:00Z");
        assert_eq!(loaded.source.as_deref(), Some("@org/my-capsule"));
    }

    #[test]
    fn meta_json_roundtrip_without_source() {
        let dir = tempfile::tempdir().unwrap();
        let meta = CapsuleMeta {
            version: "1.0.0".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            source: None,
        };
        write_meta(dir.path(), &meta).unwrap();
        let loaded = read_meta(dir.path()).expect("meta should be readable");
        assert!(loaded.source.is_none());
        // Also verify source field is omitted from JSON (skip_serializing_if)
        let json = std::fs::read_to_string(dir.path().join("meta.json")).unwrap();
        assert!(
            !json.contains("source"),
            "source: None should be omitted from JSON"
        );
    }

    #[test]
    fn read_meta_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_meta(dir.path()).is_none());
    }

    #[test]
    fn install_writes_meta_json() {
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"meta-test\"\nversion = \"2.0.0\"\n",
        )
        .unwrap();

        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        install_from_local_path(base, false, &home).expect("install should succeed");

        let installed = home.capsules_dir().join("meta-test");
        let meta = read_meta(&installed).expect("meta.json should exist after install");
        assert_eq!(meta.version, "2.0.0");
    }

    #[test]
    fn install_detects_upgrade_preserves_installed_at() {
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"upgrade-test\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());

        // First install
        install_from_local_path(base, false, &home).expect("first install");
        let meta1 = read_meta(&home.capsules_dir().join("upgrade-test")).unwrap();
        assert_eq!(meta1.version, "1.0.0");
        let original_installed_at = meta1.installed_at.clone();

        // Upgrade
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"upgrade-test\"\nversion = \"2.0.0\"\n",
        )
        .unwrap();
        install_from_local_path(base, false, &home).expect("upgrade");

        let meta2 = read_meta(&home.capsules_dir().join("upgrade-test")).unwrap();
        assert_eq!(meta2.version, "2.0.0");
        assert_eq!(
            meta2.installed_at, original_installed_at,
            "installed_at should be preserved across upgrades"
        );
    }
}
