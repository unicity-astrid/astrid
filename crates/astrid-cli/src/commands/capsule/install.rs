//! Capsule management commands - install capsules securely.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, bail};

use astrid_capsule::discovery::load_manifest;
use astrid_core::dirs::AstridHome;

pub(crate) fn install_capsule(source: &str, workspace: bool) -> anyhow::Result<()> {
    let home = AstridHome::resolve()?;

    // 1. Explicit Local Path
    if source.starts_with('.') || source.starts_with('/') {
        return install_from_local(source, workspace, &home);
    }

    // 2. OpenClaw Explicit Prefix
    if let Some(rest) = source.strip_prefix("openclaw:") {
        // If it uses the github namespace alias after the prefix
        if let Some(repo) = rest.strip_prefix('@') {
            let url = format!("https://github.com/{repo}");
            return install_from_github(&url, workspace, &home, true);
        }
        return install_from_openclaw(rest, workspace, &home);
    }

    // 3. Native Namespace Alias (@org/repo) -> GitHub
    if let Some(repo) = source.strip_prefix('@') {
        let url = format!("https://github.com/{repo}");
        return install_from_github(&url, workspace, &home, false);
    }

    // 4. Raw GitHub URL
    if source.starts_with("github.com/") || source.starts_with("https://github.com/") {
        return install_from_github(source, workspace, &home, false);
    }

    // 5. Fallback: Assume it's a local folder matching the given name
    install_from_local(source, workspace, &home)
}

pub(crate) fn install_from_github(
    url: &str,
    workspace: bool,
    home: &AstridHome,
    _is_openclaw: bool,
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

                return unpack_and_install(&download_path, workspace, home);
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
            return unpack_and_install(&entry.path(), workspace, home);
        }
    }

    bail!("Universal Migrator failed to produce a .capsule archive.");
}

pub(crate) fn install_from_openclaw(
    source: &str,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    let plugin_name = source.strip_prefix("openclaw:").unwrap_or(source);

    // Step 1: Mock Registry Fetch
    // In a real implementation, this would hit https://registry.openclaw.io
    // For now, we assume the user might have a local directory with that name for testing,
    // or we just bail if it doesn't exist locally as a fallback.
    let source_path = Path::new(plugin_name);
    if !source_path.exists() {
        bail!(
            "OpenClaw registry fetch not yet implemented. Please provide a local path to the OpenClaw plugin directory."
        );
    }

    transpile_and_install(source_path, workspace, home)
}

pub(crate) fn transpile_and_install(
    source_path: &Path,
    workspace: bool,
    home: &AstridHome,
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
    install_from_local_path(output_dir, workspace, home)
}

pub(crate) fn install_from_local(
    source: &str,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    let source_path = Path::new(source);
    if !source_path.exists() {
        bail!("Source path does not exist: {source}");
    }

    // Auto-detect OpenClaw
    if source_path.join("openclaw.plugin.json").exists()
        && !source_path.join("Capsule.toml").exists()
    {
        return transpile_and_install(source_path, workspace, home);
    }

    // Unpack .capsule archive if it is a file
    if source_path.is_file() && source.ends_with(".capsule") {
        return unpack_and_install(source_path, workspace, home);
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
                return unpack_and_install(&entry.path(), workspace, home);
            }
        }
        bail!("Failed to auto-build capsule from Cargo project.");
    }

    install_from_local_path(source_path, workspace, home)
}

fn unpack_and_install(
    archive_path: &Path,
    workspace: bool,
    home: &AstridHome,
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

    install_from_local_path(unpack_dir, workspace, home)
}

pub(crate) fn install_from_local_path(
    source_path: &Path,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    let manifest_path = source_path.join("Capsule.toml");
    if !manifest_path.exists() {
        bail!("No Capsule.toml found in {}", source_path.display());
    }

    let manifest = load_manifest(&manifest_path).context("failed to load Capsule manifest")?;
    let id = manifest.package.name.clone();

    // 3. Resolve Target Directory
    let target_dir = resolve_target_dir(home, &id, workspace)?;
    let parent = target_dir.parent().context("target dir has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    // 4. Copy Capsule

    // Backup existing target if present.
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)?;
    }

    copy_plugin_dir(source_path, &target_dir)?;

    Ok(())
}

/// Recursively copy a directory tree.
pub(crate) fn copy_plugin_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_symlink() {
            bail!(
                "plugin source contains a symlink at {}, which is not allowed",
                src_path.display()
            );
        }

        if file_type.is_dir() {
            let name = entry.file_name();
            if name == ".git" || name == "dist" || name == "target" {
                continue;
            }
            copy_plugin_dir(&src_path, &dst_path)?;
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
        Ok(root.join(".astrid").join("plugins").join(id))
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
    fn copy_plugin_dir_skips_git_and_build_artifacts() {
        let src_dir = tempfile::tempdir().unwrap();
        let base = src_dir.path();

        std::fs::write(base.join("index.js"), "// code").unwrap();
        std::fs::create_dir_all(base.join(".git/objects")).unwrap();
        std::fs::write(base.join(".git/objects/abc"), "blob").unwrap();
        std::fs::create_dir_all(base.join("dist")).unwrap();
        std::fs::write(base.join("dist/out.js"), "// built").unwrap();
        std::fs::create_dir_all(base.join("target")).unwrap();
        std::fs::write(base.join("target/debug"), "// rust").unwrap();

        let dst_dir = tempfile::tempdir().unwrap();
        copy_plugin_dir(base, dst_dir.path()).unwrap();

        assert!(dst_dir.path().join("index.js").exists());
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
}
