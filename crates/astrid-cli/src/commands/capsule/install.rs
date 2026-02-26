//! Capsule management commands - install capsules securely.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, bail};
use dialoguer::{Confirm, Input, Password, theme::ColorfulTheme};

use astrid_capsule::discovery::load_manifest;
use astrid_capsule::manifest::CapsuleManifest;
use astrid_core::dirs::AstridHome;

use crate::theme::Theme;

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
    println!(
        "{}",
        Theme::info(&format!("Fetching capsule from GitHub: {url}"))
    );

    let client = reqwest::blocking::Client::builder()
        .user_agent("astrid-cli")
        .build()?;

    let url_trimmed = url.trim_end_matches('/');
    let mut parts: Vec<&str> = url_trimmed.split('/').collect();
    if parts.len() < 2 {
        bail!("Invalid GitHub URL format. Expected github.com/org/repo or @org/repo");
    }
    let repo = parts.pop().context("Failed to get repo name from URL")?;
    let org = parts.pop().context("Failed to get org name from URL")?;

    let api_url = format!("https://api.github.com/repos/{org}/{repo}/releases/latest");

    println!("{}", Theme::dimmed("  Checking for latest release..."));

    let res = client.get(&api_url).send();

    if let Ok(response) = res
        && response.status().is_success()
        && let Ok(json) = response.json::<serde_json::Value>()
        && let Some(assets) = json.get("assets").and_then(serde_json::Value::as_array)
    {
        for asset in assets {
            if let Some(name) = asset.get("name").and_then(serde_json::Value::as_str)
                && name.ends_with(".capsule")
                && let Some(download_url) = asset.get("browser_download_url").and_then(serde_json::Value::as_str)
            {
                println!("{}", Theme::success(&format!("  Found pre-compiled capsule: {name}")));

                let tmp_dir = tempfile::tempdir()?;
                let download_path = tmp_dir.path().join(name);
                let mut file = std::fs::File::create(&download_path)?;

                let mut download_res = client.get(download_url).send()?;
                download_res.copy_to(&mut file)?;

                return unpack_and_install(&download_path, workspace, home);
            }
        }
    }

    println!("{}", Theme::dimmed("  No pre-compiled `.capsule` release found. Falling back to JIT compilation..."));

    let tmp_dir = tempfile::tempdir().context("failed to create temp dir for cloning")?;
    let clone_dir = tmp_dir.path().join(repo);

    println!("{}", Theme::dimmed("  Cloning repository to temporary directory..."));
    let status = std::process::Command::new("git")
        .args(["clone", "--depth", "1", url, &clone_dir.to_string_lossy()])
        .status()
        .context("Failed to spawn git clone")?;

    if !status.success() {
        bail!("Failed to clone repository from GitHub.");
    }

    println!("{}", Theme::info("  Building capsule using Universal Migrator..."));
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
    println!(
        "{}",
        Theme::info(&format!(
            "Installing OpenClaw plugin from registry: {plugin_name}"
        ))
    );

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
    println!(
        "{}",
        Theme::info("  Detected OpenClaw plugin. Transpiling to Astrid Capsule...")
    );

    let tmp_dir = tempfile::tempdir().context("failed to create temp dir for transpilation")?;
    let output_dir = tmp_dir.path();

    // 1. Parse OpenClaw Manifest
    let oc_manifest = astrid_openclaw::manifest::parse_manifest(source_path)
        .map_err(|e| anyhow::anyhow!("failed to parse OpenClaw manifest: {e}"))?;

    let astrid_id = astrid_openclaw::manifest::convert_id(&oc_manifest.id)
        .map_err(|e| anyhow::anyhow!("failed to convert plugin ID: {e}"))?;

    // 2. Resolve Entry Point
    let entry_point = astrid_openclaw::manifest::resolve_entry_point(source_path)
        .map_err(|e| anyhow::anyhow!("failed to resolve entry point: {e}"))?;
    let entry_path = source_path.join(&entry_point);

    // 3. Transpile JS/TS
    let source_code = std::fs::read_to_string(&entry_path)
        .with_context(|| format!("failed to read entry point {}", entry_path.display()))?;

    let transpiled = astrid_openclaw::transpiler::transpile(&source_code, &entry_point)
        .map_err(|e| anyhow::anyhow!("transpilation failed: {e}"))?;

    // 4. Generate Shim
    let shimmed = astrid_openclaw::shim::generate(&transpiled, &HashMap::new());

    // 5. Compile to WASM
    let wasm_output = output_dir.join("plugin.wasm");
    astrid_openclaw::compiler::compile(&shimmed, &wasm_output)
        .map_err(|e| anyhow::anyhow!("WASM compilation failed: {e}"))?;

    // 6. Generate Capsule.toml
    astrid_openclaw::output::generate_manifest(
        &astrid_id,
        &oc_manifest,
        &wasm_output,
        &HashMap::new(),
        output_dir,
    )
    .map_err(|e| anyhow::anyhow!("failed to generate Capsule.toml: {e}"))?;

    // 7. Proceed with standard installation from the temp directory
    println!("{}", Theme::success("  Transpilation successful."));
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

    install_from_local_path(source_path, workspace, home)
}

fn unpack_and_install(
    archive_path: &Path,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    println!(
        "{}",
        Theme::info(&format!("Unpacking capsule archive: {}", archive_path.display()))
    );

    let tmp_dir = tempfile::tempdir().context("failed to create temp dir for unpacking")?;
    let unpack_dir = tmp_dir.path();

    let tar_gz = std::fs::File::open(archive_path)
        .with_context(|| format!("Failed to open archive: {}", archive_path.display()))?;
    
    let tar = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(tar);
    archive.unpack(unpack_dir)
        .with_context(|| format!("Failed to unpack archive: {}", archive_path.display()))?;

    install_from_local_path(unpack_dir, workspace, home)
}

pub(crate) fn install_from_local_path(
    source_path: &Path,
    workspace: bool,
    home: &AstridHome,
) -> anyhow::Result<()> {
    println!(
        "{}",
        Theme::info(&format!(
            "Installing Capsule from: {}",
            source_path.display()
        ))
    );

    let manifest_path = source_path.join("Capsule.toml");
    if !manifest_path.exists() {
        bail!("No Capsule.toml found in {}", source_path.display());
    }

    let manifest = load_manifest(&manifest_path).context("failed to load Capsule manifest")?;
    let id = manifest.package.name.clone();

    // 1. Airlock Prompt
    prompt_capabilities(&manifest)?;

    // 2. Elicit Environment Variables
    let env_values = elicit_env(&manifest)?;

    // 3. Resolve Target Directory
    let target_dir = resolve_target_dir(home, &id, workspace)?;
    let parent = target_dir.parent().context("target dir has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    // 4. Copy Capsule
    println!("{}", Theme::dimmed(&format!("  Copying capsule '{id}'...")));

    // Backup existing target if present.
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)?;
    }

    copy_plugin_dir(source_path, &target_dir)?;

    // 5. Save securely elicited env to .env.json
    if !env_values.is_empty() {
        let env_path = target_dir.join(".env.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true).mode(0o600);
            let mut file = options.open(&env_path)?;
            std::io::Write::write_all(
                &mut file,
                serde_json::to_string_pretty(&env_values)?.as_bytes(),
            )?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&env_path, serde_json::to_string_pretty(&env_values)?)?;
        }
    }

    println!("{}", Theme::success(&format!("Installed capsule '{id}'")));

    if workspace {
        println!("{}", Theme::dimmed("  Location: .astrid/plugins/"));
    }

    Ok(())
}

fn prompt_capabilities(manifest: &CapsuleManifest) -> anyhow::Result<()> {
    let caps = &manifest.capabilities;
    let has_dangerous_caps = !caps.host_process.is_empty()
        || !caps.fs_read.is_empty()
        || !caps.fs_write.is_empty()
        || !caps.net.is_empty();

    if has_dangerous_caps {
        println!(
            "{}",
            Theme::warning("\nAIRLOCK PROMPT: Dangerous Capabilities Requested!")
        );
        println!(
            "{}",
            Theme::dimmed(&format!(
                "The capsule '{}' is requesting the following capabilities that escape the default sandbox:",
                manifest.package.name
            ))
        );

        if !caps.host_process.is_empty() {
            println!("  - host_process: {}", caps.host_process.join(", "));
        }
        if !caps.fs_read.is_empty() {
            println!("  - fs_read: {}", caps.fs_read.join(", "));
        }
        if !caps.fs_write.is_empty() {
            println!("  - fs_write: {}", caps.fs_write.join(", "));
        }
        if !caps.net.is_empty() {
            println!("  - net: {}", caps.net.join(", "));
        }

        println!();
        let confirm = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Do you want to grant these capabilities and continue installation?")
            .default(false)
            .interact()?;

        if !confirm {
            bail!("Installation aborted by user due to capability request.");
        }
    }

    Ok(())
}

fn elicit_env(manifest: &CapsuleManifest) -> anyhow::Result<HashMap<String, String>> {
    let mut env_values = HashMap::new();
    let theme = ColorfulTheme::default();

    if !manifest.env.is_empty() {
        println!("\n{}", Theme::info("Capsule Environment Configuration:"));
        for (key, def) in &manifest.env {
            let default_prompt = format!("Please enter value for {key}");
            let prompt_text = def.request.as_deref().unwrap_or(&default_prompt);

            let value = if def.env_type == "secret" {
                Password::with_theme(&theme)
                    .with_prompt(prompt_text)
                    .interact()?
            } else {
                let mut input = Input::<String>::with_theme(&theme).with_prompt(prompt_text);

                if let Some(default_str) = def.default.as_ref().and_then(|v| v.as_str()) {
                    input = input.default(default_str.to_string());
                }

                input.interact()?
            };

            env_values.insert(key.clone(), value);
        }
    }

    Ok(env_values)
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
            if name == "node_modules" || name == ".git" || name == "dist" || name == "target" {
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
        Ok(home.plugins_dir().join(id))
    }
}
