//! Capsule management commands - install capsules securely.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, bail};
use astrid_capsule::discovery::load_manifest;
use astrid_core::dirs::AstridHome;

use super::meta::{BakedTopic, CapsuleMeta, read_meta, write_meta};

/// Result of checking a remote source for a newer capsule version.
enum UpdateCheck {
    /// A newer version is available remotely.
    Available { latest: semver::Version },
    /// The installed version is already the latest (or newer).
    UpToDate { latest: semver::Version },
    /// Version check failed due to a transient or unexpected error.
    Failed { reason: String },
    /// Source type does not support remote version checking (expected, not an error).
    Skipped { reason: String },
}

/// Strip common version prefixes (`v`, `V`) from a Git tag before semver parsing.
fn strip_version_prefix(tag: &str) -> &str {
    tag.strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag)
}

/// Extract `(org, repo)` from a GitHub URL like `https://github.com/org/repo`,
/// `github.com/org/repo`, or `github.com/org/repo.git`. Anchors on the
/// `github.com/` marker so extra path segments (`/tree/main`, `.git`) are
/// safely ignored.
fn extract_github_org_repo(url: &str) -> Option<(&str, &str)> {
    let idx = url.find("github.com/")?;
    let after_host = &url[idx.saturating_add("github.com/".len())..];
    let trimmed = after_host.trim_end_matches('/');
    let (org, rest) = trimmed.split_once('/')?;
    // Take the first path segment as repo, stripping `.git` suffix if present
    let repo = rest.split('/').next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if org.is_empty() || repo.is_empty() {
        return None;
    }
    Some((org, repo))
}

/// Parse a capsule source string into `(org, repo)` for GitHub-backed sources.
///
/// Handles `@org/repo`, `openclaw:@org/repo`, `github.com/org/repo`, and
/// `https://github.com/org/repo`.
fn parse_github_source(source: &str) -> Option<(String, String)> {
    // @org/repo -> github.com/org/repo
    if let Some(repo_path) = source.strip_prefix('@') {
        let parts: Vec<&str> = repo_path.splitn(2, '/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Some((parts[0].to_string(), parts[1].to_string()));
        }
        return None;
    }

    // openclaw:@org/repo -> extract from the @org/repo part
    if let Some(rest) = source.strip_prefix("openclaw:") {
        if let Some(repo_path) = rest.strip_prefix('@') {
            let parts: Vec<&str> = repo_path.splitn(2, '/').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                return Some((parts[0].to_string(), parts[1].to_string()));
            }
        }
        // openclaw:name (no @) - not a GitHub source
        return None;
    }

    // github.com/org/repo or https://github.com/org/repo
    if source.contains("github.com/") {
        let (org, repo) = extract_github_org_repo(source)?;
        return Some((org.to_string(), repo.to_string()));
    }

    None
}

/// Fetch the latest release version from GitHub for a given org/repo.
fn fetch_github_latest_version(
    client: &reqwest::blocking::Client,
    org: &str,
    repo: &str,
) -> anyhow::Result<semver::Version> {
    let api_url = format!("https://api.github.com/repos/{org}/{repo}/releases/latest");
    let response = client
        .get(&api_url)
        .send()
        .context("failed to reach GitHub API")?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("no GitHub releases found for {org}/{repo}");
    }
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        bail!("GitHub API rate limit exceeded - try again later");
    }
    if !response.status().is_success() {
        bail!("GitHub API returned {}", response.status());
    }

    let json: serde_json::Value = response
        .json()
        .context("failed to parse GitHub API response")?;
    let tag_name = json
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("GitHub release has missing or empty tag_name"))?;

    let version_str = strip_version_prefix(tag_name);
    semver::Version::parse(version_str)
        .with_context(|| format!("GitHub tag '{tag_name}' is not valid semver"))
}

/// Check whether a newer version is available from a capsule's source.
fn check_remote_version(
    client: &reqwest::blocking::Client,
    source: &str,
    current_version: &str,
) -> UpdateCheck {
    let Ok(current) = semver::Version::parse(current_version) else {
        return UpdateCheck::Failed {
            reason: format!("installed version '{current_version}' is not valid semver"),
        };
    };

    // OpenClaw (non-GitHub) sources - expected, not an error
    if source.starts_with("openclaw:") && !source.contains('@') {
        return UpdateCheck::Skipped {
            reason: "OpenClaw registry not yet implemented".to_string(),
        };
    }

    // Local paths cannot be version-checked remotely - expected, not an error
    if source.starts_with('.') || source.starts_with('/') {
        return UpdateCheck::Skipped {
            reason: "local source".to_string(),
        };
    }

    // GitHub-backed sources
    if let Some((org, repo)) = parse_github_source(source) {
        match fetch_github_latest_version(client, &org, &repo) {
            Ok(latest) => {
                if latest > current {
                    UpdateCheck::Available { latest }
                } else {
                    UpdateCheck::UpToDate { latest }
                }
            },
            Err(e) => UpdateCheck::Failed {
                reason: format!("{e}"),
            },
        }
    } else {
        UpdateCheck::Skipped {
            reason: format!("unsupported source: {source}"),
        }
    }
}

/// Update one or all installed capsules from their original source.
///
/// If `target` is `Some`, force-reinstall that capsule from its recorded source.
/// If `None`, check all installed capsules for newer versions and only update
/// those where the remote version is strictly newer (semver comparison).
///
/// # TODO
/// - Add a registry manifest (like brew formulas) that pins version + Blake3 hash
///   per capsule. `update` should fetch the manifest, compare versions against
///   `meta.json`, only download if newer, and verify Blake3 hash before installing.
///   Trust chain: registry manifest (signed) -> pinned URL + Blake3 -> verified binary.
pub(crate) fn update_capsule(target: Option<&str>, workspace: bool) -> anyhow::Result<()> {
    let home = AstridHome::resolve()?;

    if let Some(name) = target {
        // Single capsule: always force-reinstall from recorded source.
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
        update_all_capsules(&home, workspace)
    }
}

/// Check all installed capsules for updates and install those with newer versions.
fn update_all_capsules(home: &AstridHome, workspace: bool) -> anyhow::Result<()> {
    let capsules_dir = home.capsules_dir();
    if !capsules_dir.exists() {
        eprintln!("No capsules installed.");
        return Ok(());
    }

    // Collect installed capsules with their metadata
    let mut capsules: Vec<(String, Option<CapsuleMeta>)> = Vec::new();
    for entry in std::fs::read_dir(&capsules_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = read_meta(&entry.path());
        capsules.push((name, meta));
    }

    if capsules.is_empty() {
        eprintln!("No capsules installed.");
        return Ok(());
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent("astrid-cli")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    eprintln!(
        "Checking {} installed capsule(s) for updates...",
        capsules.len()
    );

    // Check phase: determine which capsules have updates available
    let mut to_update: Vec<(String, String)> = Vec::new(); // (name, source)
    let mut up_to_date = 0u32;
    let mut check_failed = 0u32;
    let mut skipped = 0u32;

    for (name, meta) in &capsules {
        let Some(meta) = meta else {
            eprintln!("  {name}: skipped (no meta.json)");
            skipped = skipped.saturating_add(1);
            continue;
        };
        let Some(ref source) = meta.source else {
            eprintln!("  {name}: skipped (no source recorded)");
            skipped = skipped.saturating_add(1);
            continue;
        };

        match check_remote_version(&client, source, &meta.version) {
            UpdateCheck::Available { latest } => {
                eprintln!("  {name}: {} -> {latest} (update available)", meta.version);
                to_update.push((name.clone(), source.clone()));
            },
            UpdateCheck::UpToDate { latest } => {
                eprintln!("  {name}: {} (up to date, latest: {latest})", meta.version);
                up_to_date = up_to_date.saturating_add(1);
            },
            UpdateCheck::Failed { reason } => {
                eprintln!("  {name}: {} (check failed: {reason})", meta.version);
                check_failed = check_failed.saturating_add(1);
            },
            UpdateCheck::Skipped { reason } => {
                eprintln!("  {name}: skipped ({reason})");
                skipped = skipped.saturating_add(1);
            },
        }
    }

    // Install phase: update capsules with newer versions
    let mut updated = 0u32;
    let mut install_failed = 0u32;
    for (name, source) in &to_update {
        eprintln!("Updating {name} from {source}...");
        if let Err(e) = install_capsule(source, workspace) {
            eprintln!("  Failed to update {name}: {e}");
            install_failed = install_failed.saturating_add(1);
        } else {
            updated = updated.saturating_add(1);
        }
    }

    eprintln!(
        "Done: {updated} updated, {up_to_date} up-to-date, {check_failed} check-failed, {skipped} skipped, {install_failed} install-failed."
    );
    Ok(())
}

pub(crate) fn install_capsule(source: &str, workspace: bool) -> anyhow::Result<()> {
    let home = AstridHome::resolve()?;

    // 1. Explicit Local Path - no source tracking (re-fetch doesn't make sense)
    if source.starts_with('.') || source.starts_with('/') {
        return install_from_local(source, workspace, &home, None);
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

    // 5. Fallback: Assume it's a local folder - no source tracking
    install_from_local(source, workspace, &home, None)
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

    let (org, repo) = extract_github_org_repo(url).ok_or_else(|| {
        anyhow::anyhow!("Invalid GitHub URL format. Expected github.com/org/repo or @org/repo")
    })?;

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

    // Bake topic declarations with inline schema content.
    let baked_topics = match bake_topics(&manifest, &target_dir) {
        Ok(t) => t,
        Err(e) => {
            // Rollback: restore previous installation (lifecycle hook already ran
            // and cannot be undone, but at least restore the directory state).
            let _ = std::fs::remove_dir_all(&target_dir);
            if let Some(ref backup) = backup_dir {
                let _ = std::fs::rename(backup, &target_dir);
            }
            return Err(e);
        },
    };

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
        provides: manifest
            .effective_provides()
            .iter()
            .filter(|cap| {
                // Auto-derived provides may contain wildcards from ipc_publish
                // patterns (e.g. "topic:registry.v1.response.*"). Filter them
                // out — meta.json provides should be concrete capabilities.
                let body = cap.split_once(':').map_or(cap.as_str(), |(_, b)| b);
                !body.contains('*')
            })
            .cloned()
            .collect(),
        requires: manifest.dependencies.requires.clone(),
        topics: baked_topics,
    };
    write_meta(&target_dir, &meta)?;

    // Clean up backup
    if let Some(ref backup) = backup_dir {
        let _ = std::fs::remove_dir_all(backup);
    }

    Ok(())
}

/// Maximum schema file size (1 MB). Prevents oversized schemas from bloating `meta.json`.
const MAX_SCHEMA_FILE_SIZE: u64 = 1024 * 1024;

/// Read topic declarations from the manifest and bake schema file content inline.
///
/// For each `[[topic]]` entry with a `schema` path, reads the JSON file from
/// `capsule_dir`, validates it as JSON, and embeds the parsed content in the
/// returned `BakedTopic`. Fails if any schema file is missing, too large, not
/// valid JSON, or escapes the capsule directory via symlinks.
fn bake_topics(
    manifest: &astrid_capsule::manifest::CapsuleManifest,
    capsule_dir: &Path,
) -> anyhow::Result<Vec<BakedTopic>> {
    let mut baked = Vec::with_capacity(manifest.topics.len());

    let canonical_capsule_dir = std::fs::canonicalize(capsule_dir).with_context(|| {
        format!(
            "failed to canonicalize capsule dir: {}",
            capsule_dir.display()
        )
    })?;

    for topic in &manifest.topics {
        let schema = if let Some(ref schema_path) = topic.schema {
            let full_path = capsule_dir.join(schema_path);

            // Resolve symlinks and verify the canonical path stays within the capsule dir.
            let canonical = std::fs::canonicalize(&full_path).with_context(|| {
                format!(
                    "[[topic]] '{}' schema file not found: '{}'",
                    topic.name,
                    full_path.display()
                )
            })?;
            if !canonical.starts_with(&canonical_capsule_dir) {
                bail!(
                    "[[topic]] '{}' schema path '{}' resolves outside the capsule directory",
                    topic.name,
                    schema_path.display()
                );
            }

            // Open once and use .take() to enforce a hard ceiling on bytes read,
            // preventing a concurrent append from bypassing the size limit.
            let file = std::fs::File::open(&canonical).with_context(|| {
                format!(
                    "failed to open schema file for topic '{}': '{}'",
                    topic.name,
                    canonical.display()
                )
            })?;
            let file_len = file
                .metadata()
                .with_context(|| format!("failed to stat schema file: {}", canonical.display()))?
                .len();
            if file_len > MAX_SCHEMA_FILE_SIZE {
                bail!(
                    "[[topic]] '{}' schema file '{}' is {} bytes, exceeding the {} byte limit",
                    topic.name,
                    schema_path.display(),
                    file_len,
                    MAX_SCHEMA_FILE_SIZE
                );
            }
            let capacity = usize::try_from(file_len)
                .expect("MAX_SCHEMA_FILE_SIZE is small enough to fit in usize");
            let mut content = String::with_capacity(capacity);
            // Read at most MAX_SCHEMA_FILE_SIZE + 1 bytes so we can detect growth.
            std::io::Read::read_to_string(
                &mut std::io::Read::take(file, MAX_SCHEMA_FILE_SIZE + 1),
                &mut content,
            )
            .with_context(|| {
                format!(
                    "failed to read schema file for topic '{}': '{}'",
                    topic.name,
                    canonical.display()
                )
            })?;
            if content.len() as u64 > MAX_SCHEMA_FILE_SIZE {
                bail!(
                    "[[topic]] '{}' schema file '{}' exceeded the {} byte limit during read",
                    topic.name,
                    schema_path.display(),
                    MAX_SCHEMA_FILE_SIZE
                );
            }
            let value: serde_json::Value = serde_json::from_str(&content).with_context(|| {
                format!(
                    "[[topic]] '{}' schema file '{}' contains invalid JSON",
                    topic.name,
                    schema_path.display()
                )
            })?;
            Some(value)
        } else {
            None
        };

        baked.push(BakedTopic {
            name: topic.name.clone(),
            direction: topic.direction,
            description: topic.description.clone(),
            schema,
        });
    }

    Ok(baked)
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
    // Exact match: the elicit host function publishes to "astrid.v1.elicit".
    // If the topic is ever extended (e.g. "astrid.v1.elicit.request"), update
    // this subscription and the integration test in lifecycle_e2e.rs.
    let elicit_receiver = event_bus.subscribe_topic("astrid.v1.elicit");
    let elicit_handle = rt.spawn(async move {
        cli_elicit_handler(elicit_receiver, elicit_bus).await;
    });

    let capsule_id_owned = astrid_capsule::capsule::CapsuleId::new(capsule_id.to_string())
        .map_err(|e| anyhow::anyhow!("invalid capsule ID: {e}"))?;
    let secret_store =
        astrid_storage::build_secret_store(capsule_id, kv.clone(), rt.handle().clone());
    let cfg = astrid_capsule::engine::wasm::LifecycleConfig {
        wasm_bytes,
        capsule_id: capsule_id_owned,
        workspace_root: target_dir.to_path_buf(),
        kv,
        event_bus: event_bus.clone(),
        config: std::collections::HashMap::new(),
        secret_store,
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

        let response_topic = format!("astrid.v1.elicit.response.{request_id}");
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
            provides: vec!["tool:run_shell".into(), "topic:session.response.*".into()],
            requires: vec!["topic:identity.response.ready".into()],
            topics: vec![],
        };
        write_meta(dir.path(), &meta).unwrap();
        let loaded = read_meta(dir.path()).expect("meta should be readable");
        assert_eq!(loaded.version, "1.2.3");
        assert_eq!(loaded.installed_at, "2026-01-01T00:00:00Z");
        assert_eq!(loaded.updated_at, "2026-03-12T00:00:00Z");
        assert_eq!(loaded.source.as_deref(), Some("@org/my-capsule"));
        assert_eq!(loaded.provides.len(), 2);
        assert_eq!(loaded.requires, vec!["topic:identity.response.ready"]);
    }

    #[test]
    fn meta_json_roundtrip_without_source() {
        let dir = tempfile::tempdir().unwrap();
        let meta = CapsuleMeta {
            version: "1.0.0".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            source: None,
            provides: vec![],
            requires: vec![],
            topics: vec![],
        };
        write_meta(dir.path(), &meta).unwrap();
        let loaded = read_meta(dir.path()).expect("meta should be readable");
        assert!(loaded.source.is_none());
        // Also verify optional fields are omitted from JSON (skip_serializing_if)
        let json = std::fs::read_to_string(dir.path().join("meta.json")).unwrap();
        assert!(
            !json.contains("source"),
            "source: None should be omitted from JSON"
        );
        assert!(
            !json.contains("provides"),
            "empty provides should be omitted from JSON"
        );
        assert!(
            !json.contains("requires"),
            "empty requires should be omitted from JSON"
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

    #[test]
    fn test_strip_version_prefix() {
        assert_eq!(strip_version_prefix("v1.2.3"), "1.2.3");
        assert_eq!(strip_version_prefix("V1.0.0"), "1.0.0");
        assert_eq!(strip_version_prefix("1.0.0"), "1.0.0");
        assert_eq!(strip_version_prefix("v0.0.1-alpha"), "0.0.1-alpha");
        // Non-standard prefixes are left as-is (semver parse will fail gracefully)
        assert_eq!(strip_version_prefix("release-1.0.0"), "release-1.0.0");
    }

    #[test]
    fn test_extract_github_org_repo() {
        let (org, repo) = extract_github_org_repo("https://github.com/org/repo").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");

        let (org, repo) = extract_github_org_repo("github.com/myorg/myrepo").unwrap();
        assert_eq!(org, "myorg");
        assert_eq!(repo, "myrepo");

        // Trailing slash
        let (org, repo) = extract_github_org_repo("https://github.com/org/repo/").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");

        // Too short
        assert!(extract_github_org_repo("singlepart").is_none());
    }

    #[test]
    fn test_parse_github_source_at_prefix() {
        let (org, repo) = parse_github_source("@org/repo").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_source_https() {
        let (org, repo) = parse_github_source("https://github.com/org/repo").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_source_bare() {
        let (org, repo) = parse_github_source("github.com/org/repo").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_source_openclaw_at() {
        let (org, repo) = parse_github_source("openclaw:@org/repo").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_source_non_github() {
        assert!(parse_github_source("openclaw:my-capsule").is_none());
        assert!(parse_github_source("./local/path").is_none());
        assert!(parse_github_source("/absolute/path").is_none());
    }

    #[test]
    fn test_check_remote_version_openclaw_skipped() {
        let client = reqwest::blocking::Client::new();
        let result = check_remote_version(&client, "openclaw:my-capsule", "1.0.0");
        assert!(matches!(result, UpdateCheck::Skipped { reason } if reason.contains("OpenClaw")));
    }

    #[test]
    fn test_check_remote_version_invalid_semver() {
        let client = reqwest::blocking::Client::new();
        let result = check_remote_version(&client, "@org/repo", "not-a-version");
        assert!(
            matches!(result, UpdateCheck::Failed { reason } if reason.contains("not valid semver"))
        );
    }

    #[test]
    fn test_check_remote_version_local_skipped() {
        let client = reqwest::blocking::Client::new();
        let result = check_remote_version(&client, "./local/path", "1.0.0");
        assert!(
            matches!(result, UpdateCheck::Skipped { reason } if reason.contains("local source"))
        );

        let result = check_remote_version(&client, "/absolute/path", "1.0.0");
        assert!(
            matches!(result, UpdateCheck::Skipped { reason } if reason.contains("local source"))
        );
    }

    #[test]
    fn install_bakes_topic_schema_into_meta() {
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();
        std::fs::create_dir_all(base.join("schemas")).unwrap();
        std::fs::write(
            base.join("schemas/chunk.json"),
            r#"{"type":"object","properties":{"content":{"type":"string"}}}"#,
        )
        .unwrap();
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"topic-test\"\nversion = \"1.0.0\"\n\n\
             [[topic]]\nname = \"llm.v1.chunk\"\ndirection = \"publish\"\n\
             description = \"Streaming chunk\"\nschema = \"schemas/chunk.json\"\n\n\
             [[topic]]\nname = \"llm.v1.request\"\ndirection = \"subscribe\"\n",
        )
        .unwrap();

        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        install_from_local_path(base, false, &home).expect("install should succeed");

        let installed = home.capsules_dir().join("topic-test");
        let meta = read_meta(&installed).expect("meta.json should exist");
        assert_eq!(meta.topics.len(), 2);
        assert_eq!(meta.topics[0].name, "llm.v1.chunk");
        assert_eq!(
            meta.topics[0].direction,
            astrid_capsule::manifest::TopicDirection::Publish
        );
        assert_eq!(
            meta.topics[0].description.as_deref(),
            Some("Streaming chunk")
        );
        assert!(meta.topics[0].schema.is_some(), "schema should be baked");
        let schema = meta.topics[0].schema.as_ref().unwrap();
        assert_eq!(schema["type"], "object");

        // Second topic has no schema
        assert_eq!(meta.topics[1].name, "llm.v1.request");
        assert_eq!(
            meta.topics[1].direction,
            astrid_capsule::manifest::TopicDirection::Subscribe
        );
        assert!(meta.topics[1].schema.is_none());
    }

    #[test]
    fn install_fails_on_missing_schema_file() {
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"missing-schema\"\nversion = \"1.0.0\"\n\n\
             [[topic]]\nname = \"foo.bar\"\ndirection = \"publish\"\n\
             schema = \"schemas/nonexistent.json\"\n",
        )
        .unwrap();

        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        let err = install_from_local_path(base, false, &home)
            .expect_err("install should fail with missing schema");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("schema file not found") || msg.contains("No such file"),
            "expected schema-file-not-found error, got: {msg}"
        );
    }

    #[test]
    fn install_fails_on_invalid_json_schema() {
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();
        std::fs::write(base.join("bad.json"), "not valid json {{{").unwrap();
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"bad-json\"\nversion = \"1.0.0\"\n\n\
             [[topic]]\nname = \"foo.bar\"\ndirection = \"publish\"\n\
             schema = \"bad.json\"\n",
        )
        .unwrap();

        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        let err = install_from_local_path(base, false, &home)
            .expect_err("install should fail with invalid JSON");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("invalid JSON"),
            "expected invalid JSON error, got: {msg}"
        );
    }

    #[test]
    fn install_fails_on_oversized_schema() {
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();
        // Create a file just over 1 MB
        let big = vec![b' '; (MAX_SCHEMA_FILE_SIZE as usize) + 1];
        std::fs::write(base.join("big.json"), &big).unwrap();
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"big-schema\"\nversion = \"1.0.0\"\n\n\
             [[topic]]\nname = \"foo.bar\"\ndirection = \"publish\"\n\
             schema = \"big.json\"\n",
        )
        .unwrap();

        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        let err = install_from_local_path(base, false, &home)
            .expect_err("install should fail with oversized schema");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("exceeding"),
            "expected size limit error, got: {msg}"
        );
    }

    #[test]
    fn install_no_topics_backwards_compat() {
        // Existing capsules without [[topic]] should still produce valid meta.json.
        let capsule_dir = tempfile::tempdir().unwrap();
        let base = capsule_dir.path();
        std::fs::write(
            base.join("Capsule.toml"),
            "[package]\nname = \"no-topics\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let home_dir = tempfile::tempdir().unwrap();
        let home = AstridHome::from_path(home_dir.path());
        install_from_local_path(base, false, &home).expect("install should succeed");

        let installed = home.capsules_dir().join("no-topics");
        let meta = read_meta(&installed).expect("meta.json should exist");
        assert!(meta.topics.is_empty());
    }

    #[test]
    fn test_extract_github_org_repo_extra_path() {
        // Extra path segments after org/repo should be ignored
        let (org, repo) = extract_github_org_repo("https://github.com/org/repo/tree/main").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_extract_github_org_repo_git_suffix() {
        let (org, repo) = extract_github_org_repo("https://github.com/org/repo.git").unwrap();
        assert_eq!(org, "org");
        assert_eq!(repo, "repo");
    }
}
