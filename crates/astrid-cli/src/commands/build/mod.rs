use crate::commands::build::archiver::pack_capsule_archive;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub(crate) mod archiver;

/// Main entrypoint for the `astrid build` Universal Packager command.
pub(crate) fn run_build(
    path: Option<&str>,
    output: Option<&str>,
    project_type: Option<&str>,
    from_mcp_json: Option<&str>,
) -> Result<()> {
    let target_dir = match path {
        Some(p) => Path::new(p).to_path_buf(),
        None => std::env::current_dir()?,
    };

    if !target_dir.exists() {
        bail!("Directory does not exist: {}", target_dir.display());
    }

    // Early exit for legacy `mcp.json` or `gemini-extension.json` quick convert
    if let Some(json_path_str) = from_mcp_json {
        let json_path = Path::new(json_path_str);
        let dir = json_path.parent().unwrap_or(Path::new(""));
        let file_name = json_path.file_name().unwrap_or_default().to_string_lossy();
        return handle_mcp_quick_convert(dir, &file_name, output);
    }

    // Step 1: Detect the project type if not explicitly provided
    let detected_type = if let Some(explicit) = project_type {
        explicit.to_string()
    } else {
        detect_project_type(&target_dir)?
    };

    info!("🔍 Detected project type: {}", detected_type);

    // Step 2: Route to the appropriate builder strategy
    match detected_type.as_str() {
        "rust" => {
            build_rust_capsule(&target_dir, output);
        }
        "mcp" => handle_mcp_quick_convert(&target_dir, "mcp.json", output)?,
        "extension" => handle_mcp_quick_convert(&target_dir, "gemini-extension.json", output)?,
        "js" | "ts" | "node" => {
            bail!("JS/TS building via AstridClaw is not yet implemented in the CLI.");
        }
        "static" => {
            bail!("Static No-Code building is not yet implemented in the CLI.");
        }
        unknown => {
            bail!("Unknown project type: {unknown}. Supported types: rust, mcp, extension");
        }
    }

    Ok(())
}

fn detect_project_type(dir: &Path) -> Result<String> {
    if dir.join("Cargo.toml").exists() {
        return Ok("rust".to_string());
    }
    
    if dir.join("gemini-extension.json").exists() {
        return Ok("extension".to_string());
    }

    if dir.join("package.json").exists() {
        return Ok("js".to_string());
    }

    if dir.join("mcp.json").exists() {
        return Ok("mcp".to_string());
    }

    // Default to looking for a naked Capsule.toml
    if dir.join("Capsule.toml").exists() {
        return Ok("static".to_string());
    }

    bail!("Could not automatically detect the project type. Please ensure a Cargo.toml, gemini-extension.json, package.json, or Capsule.toml exists in the directory, or use the --type flag.");
}

fn build_rust_capsule(_dir: &Path, _output: Option<&str>) {
    // TODO: Implement the `cargo build --target wasm32-wasip1`, Extism schema extraction, and tarball packing
    warn!("Rust builder is currently a stub.");
}

#[allow(dead_code)]
fn build_interactive_mcp_capsule(_dir: &Path, _output: Option<&str>) {
    // TODO: Implement the interactive dialoguer wizard for Legacy MCP
    warn!("Interactive MCP builder is currently a stub.");
}

#[allow(clippy::too_many_lines)]
fn handle_mcp_quick_convert(dir: &Path, json_filename: &str, output: Option<&str>) -> Result<()> {
    let json_path = dir.join(json_filename);
    info!("🔄 Converting {} into a Universal Capsule...", json_path.display());
    
    let content = fs::read_to_string(&json_path)
        .with_context(|| format!("Failed to read {}", json_path.display()))?;
    
    let parsed: Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON in {}", json_path.display()))?;

    // 1. Extract Package Metadata
    let name = parsed.get("name").and_then(Value::as_str).unwrap_or("legacy-mcp").to_string();
    let version = parsed.get("version").and_then(Value::as_str).unwrap_or("1.0.0").to_string();
    let description = parsed.get("description").and_then(Value::as_str).unwrap_or("Converted MCP capsule").to_string();

    let mut toml = String::new();
    let _ = write!(toml, "[package]\nname = \"{name}\"\nversion = \"{version}\"\ndescription = \"{description}\"\nauthors = [\"Auto-Converter\"]\n\n");

    let mut additional_files = Vec::new();

    // 2. Extract settings and convert to `[env]` block (gemini-extension.json specific)
    let mut has_env = false;
    let mut env_block = String::from("[env]\n");

    if let Some(settings) = parsed.get("settings").and_then(Value::as_array) {
        for setting in settings {
            if let Some(env_var) = setting.get("envVar").and_then(Value::as_str) {
                has_env = true;
                let req_name = setting.get("name").and_then(Value::as_str).unwrap_or(env_var);
                let desc = setting.get("description").and_then(Value::as_str).unwrap_or("");
                
                let _ = writeln!(env_block, "{env_var} = {{ type = \"secret\", request = \"{req_name}\", description = \"{desc}\" }}");
            }
        }
    }

    // Fallback: If no `settings` block, but we find `env` inside the mcpServers, we strip them and ask generically.
    if !has_env
        && let Some(servers) = parsed.get("mcpServers").and_then(Value::as_object)
    {
        for (_, server_config) in servers {
            if let Some(env_map) = server_config.get("env").and_then(Value::as_object) {
                for (env_key, _) in env_map {
                    has_env = true;
                    let _ = writeln!(env_block, "{env_key} = {{ type = \"secret\", request = \"Please provide a value for {env_key}\" }}");
                }
            }
        }
    }

    // 3. Extract capabilities (if any commands are defined)
    let mut capabilities = Vec::new();
    if let Some(servers) = parsed.get("mcpServers").and_then(Value::as_object) {
        for (_, server_config) in servers {
            if let Some(cmd) = server_config.get("command").and_then(Value::as_str)
                && !capabilities.contains(&cmd.to_string())
            {
                capabilities.push(cmd.to_string());
            }
        }
    }

    if !capabilities.is_empty() {
        toml.push_str("[capabilities]\n");
        toml.push_str("host_process = [");
        let formatted_caps: Vec<String> = capabilities.iter().map(|c| format!("\"{c}\"")).collect();
        toml.push_str(&formatted_caps.join(", "));
        toml.push_str("]\n\n");
    }

    if has_env {
        toml.push_str(&env_block);
        toml.push('\n');
    }

    // 4. Extract MCP Servers
    if let Some(servers) = parsed.get("mcpServers").and_then(Value::as_object) {
        for (server_id, server_config) in servers {
            toml.push_str("[[mcp_server]]\n");
            let _ = writeln!(toml, "id = \"{server_id}\"");
            
            if let Some(desc) = server_config.get("description").and_then(Value::as_str) {
                let _ = writeln!(toml, "description = \"{desc}\"");
            }

            if let Some(cmd) = server_config.get("command").and_then(Value::as_str) {
                toml.push_str("type = \"stdio\"\n");
                let _ = writeln!(toml, "command = \"{cmd}\"");
                
                if let Some(args) = server_config.get("args").and_then(Value::as_array) {
                    let formatted_args: Vec<String> = args.iter()
                        .filter_map(Value::as_str)
                        .map(|a| format!("\"{a}\""))
                        .collect();
                    if !formatted_args.is_empty() {
                        let joined = formatted_args.join(", ");
                        let _ = writeln!(toml, "args = [{joined}]");
                    }
                }
            } else if let Some(http_url) = server_config.get("httpUrl").and_then(Value::as_str) {
                toml.push_str("type = \"sse\"\n");
                let _ = writeln!(toml, "url = \"{http_url}\"");
            }
            toml.push('\n');
        }
    }

    // 5. Inject Context Files (GEMINI.md)
    let context_file_name = parsed.get("contextFileName").and_then(Value::as_str).unwrap_or("GEMINI.md");
    let context_path = dir.join(context_file_name);
    if context_path.exists() {
        let _ = write!(toml, "[[context_file]]\nname = \"workspace-context\"\nfile = \"{context_file_name}\"\n\n");
        additional_files.push(context_path);
    }

    // 6. Inject Skills
    let skills_dir = dir.join("skills");
    if skills_dir.exists() && skills_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&skills_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let skill_name = path.file_stem().unwrap_or_default().to_string_lossy();
                let _ = write!(toml, "[[skill]]\nname = \"{skill_name}\"\nfile = \"skills/{file_name}\"\n\n");
            }
        }
        additional_files.push(skills_dir);
    }

    // 7. Inject Commands
    let commands_dir = dir.join("commands");
    if commands_dir.exists() && commands_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&commands_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("toml") {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let cmd_name = format!("/{}", path.file_stem().unwrap_or_default().to_string_lossy());
                let _ = write!(toml, "[[command]]\nname = \"{cmd_name}\"\nfile = \"commands/{file_name}\"\n\n");
            }
        }
        additional_files.push(commands_dir);
    }

    // 8. Pack the archive
    let out_dir = match output {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?.join("dist"),
    };
    
    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)?;
    }

    let out_file = out_dir.join(format!("{name}.capsule"));
    let refs: Vec<&Path> = additional_files.iter().map(std::path::PathBuf::as_path).collect();
    
    pack_capsule_archive(&out_file, &toml, None, dir, &refs)?;

    info!("🎉 Successfully converted to universal capsule: {}", out_file.display());
    Ok(())
}
