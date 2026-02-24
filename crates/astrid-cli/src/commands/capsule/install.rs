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
    install_from_local(source, workspace, &home)
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

    println!(
        "{}",
        Theme::info(&format!("Installing Capsule from local path: {source}"))
    );

    let manifest_path = source_path.join("Capsule.toml");
    if !manifest_path.exists() {
        bail!("No Capsule.toml found in {source}");
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
        std::fs::write(&env_path, serde_json::to_string_pretty(&env_values)?)?;
    }

    println!("{}", Theme::success(&format!("Installed capsule '{id}'")));

    if workspace {
        println!("{}", Theme::dimmed("  Location: .astrid/plugins/"));
    }

    Ok(())
}

fn prompt_capabilities(manifest: &CapsuleManifest) -> anyhow::Result<()> {
    let caps = &manifest.capabilities;
    let has_dangerous_caps =
        !caps.host_process.is_empty() || !caps.fs_read.is_empty() || !caps.fs_write.is_empty();

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
            if name == "node_modules" || name == ".git" || name == "dist" {
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
