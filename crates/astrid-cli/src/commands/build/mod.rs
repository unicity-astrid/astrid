use crate::commands::build::archiver::pack_capsule_archive;
use anyhow::{Context, Result, bail};
use serde_json::Value;
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
        "rust" => build_rust_capsule(&target_dir, output)?,
        "mcp" => handle_mcp_quick_convert(&target_dir, "mcp.json", output)?,
        "extension" => handle_mcp_quick_convert(&target_dir, "gemini-extension.json", output)?,
        "js" | "ts" | "node" => {
            bail!("JS/TS building via AstridClaw is not yet implemented in the CLI.");
        },
        "static" => {
            bail!("Static No-Code building is not yet implemented in the CLI.");
        },
        unknown => {
            bail!("Unknown project type: {unknown}. Supported types: rust, mcp, extension");
        },
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

    bail!(
        "Could not automatically detect the project type. Please ensure a Cargo.toml, gemini-extension.json, package.json, or Capsule.toml exists in the directory, or use the --type flag."
    );
}

fn create_dummy_functions() -> impl IntoIterator<Item = extism::Function> {
    use extism::{Function, UserData};

    let dummy = |name: &str, num_inputs: usize, num_outputs: usize| {
        let inputs = vec![extism::PTR; num_inputs];
        let outputs = vec![extism::PTR; num_outputs];
        Function::new(name, inputs, outputs, UserData::new(()), |_, _, _, _| {
            Err(extism::Error::msg("Dummy function called"))
        })
    };

    vec![
        dummy("astrid_fs_exists", 1, 1),
        dummy("astrid_fs_mkdir", 1, 0),
        dummy("astrid_fs_readdir", 1, 1),
        dummy("astrid_fs_stat", 1, 1),
        dummy("astrid_fs_unlink", 1, 0),
        dummy("astrid_read_file", 1, 1),
        dummy("astrid_write_file", 2, 0),
        dummy("astrid_ipc_publish", 2, 0),
        dummy("astrid_ipc_subscribe", 1, 1),
        dummy("astrid_ipc_unsubscribe", 1, 0),
        dummy("astrid_ipc_poll", 1, 1),
        dummy("astrid_uplink_register", 3, 1),
        dummy("astrid_uplink_send", 3, 1),
        dummy("astrid_kv_get", 1, 1),
        dummy("astrid_kv_set", 2, 0),
        dummy("astrid_get_config", 1, 1),
        dummy("astrid_http_request", 1, 1),
        dummy("astrid_log", 2, 0),
        dummy("astrid_cron_schedule", 3, 0),
        dummy("astrid_cron_cancel", 1, 0),
        dummy("astrid_spawn_host", 1, 1),
    ]
}

#[allow(clippy::too_many_lines)]
fn build_rust_capsule(dir: &Path, output: Option<&str>) -> Result<()> {
    info!("🔨 Building Rust WASM capsule from {}", dir.display());

    // 1. Verify cargo is available
    let cargo_check = std::process::Command::new("cargo")
        .arg("--version")
        .output();
    if cargo_check.is_err() {
        bail!("`cargo` is not installed or not in PATH. Rust compilation failed.");
    }

    // 2. Parse Cargo Metadata to get the exact artifact name
    let meta = cargo_metadata::MetadataCommand::new()
        .current_dir(dir)
        .no_deps()
        .exec()
        .context("Failed to parse Cargo metadata")?;

    let package = meta
        .root_package()
        .context("No root package found in Cargo.toml")?;
    let crate_name = package.name.clone();
    let package_version = package.version.to_string();
    let wasm_name = crate_name.replace('-', "_");

    // 3. Compile the WASM target
    info!("   Compiling target wasm32-wasip1...");
    let status = std::process::Command::new("cargo")
        .current_dir(dir)
        .args(["build", "--target", "wasm32-wasip1", "--release"])
        .status()
        .context("Failed to spawn cargo build")?;

    if !status.success() {
        bail!(
            "Cargo build failed. Ensure you have the target installed: `rustup target add wasm32-wasip1`"
        );
    }

    // 4. Locate the compiled WASM binary
    // Assuming a standard target directory structure for single packages or excluded workspace members
    let mut wasm_path = dir
        .join("target")
        .join("wasm32-wasip1")
        .join("release")
        .join(format!("{wasm_name}.wasm"));

    // Fallback: Check the global workspace target directory if it wasn't built locally
    if !wasm_path.exists() {
        wasm_path = meta
            .workspace_root
            .into_std_path_buf()
            .join("target")
            .join("wasm32-wasip1")
            .join("release")
            .join(format!("{wasm_name}.wasm"));
    }

    if !wasm_path.exists() {
        bail!(
            "Could not locate compiled WASM binary at {}",
            wasm_path.display()
        );
    }

    // 5. Extract Schemas using Extism
    info!("   Extracting Extism schemas...");
    let wasm_bytes = fs::read(&wasm_path).context("Failed to read compiled WASM binary")?;
    let manifest = extism::Manifest::new([extism::Wasm::data(wasm_bytes)]);
    let mut plugin = extism::Plugin::new(&manifest, create_dummy_functions(), true)
        .context("Failed to initialize Extism plugin for schema extraction")?;

    let schema_json = match plugin.call::<(), String>("astrid_export_schemas", ()) {
        Ok(json) => json,
        Err(e) => {
            warn!(
                "Capsule does not export schemas (astrid_export_schemas failed: {}). Proceeding without auto-generated tools.",
                e
            );
            "{}".to_string()
        },
    };

    let extracted_tools: Value = serde_json::from_str(&schema_json)
        .unwrap_or_else(|_| Value::Object(serde_json::Map::default()));

    // 6. Merge with developer's Capsule.toml
    let base_toml_path = dir.join("Capsule.toml");
    let mut toml_doc = if base_toml_path.exists() {
        let content = fs::read_to_string(&base_toml_path).context("Failed to read Capsule.toml")?;
        content
            .parse::<toml_edit::DocumentMut>()
            .context("Failed to parse Capsule.toml")?
    } else {
        let mut doc = toml_edit::DocumentMut::new();

        let mut package = toml_edit::Table::new();
        package.insert("name", toml_edit::value(crate_name.as_str()));
        package.insert("version", toml_edit::value(package_version));
        package.insert("description", toml_edit::value(""));
        doc.insert("package", toml_edit::Item::Table(package));

        let mut comp = toml_edit::Table::new();
        comp.insert("id", toml_edit::value(crate_name.as_str()));
        comp.insert("file", toml_edit::value(format!("{wasm_name}.wasm")));
        comp.insert("type", toml_edit::value("executable"));

        let mut comp_arr = toml_edit::ArrayOfTables::new();
        comp_arr.push(comp);

        doc.insert("component", toml_edit::Item::ArrayOfTables(comp_arr));

        doc
    };

    // Inject the tools
    if let Value::Object(tools) = extracted_tools
        && !tools.is_empty()
    {
        // Get or create the `tool` array of tables
        let mut tools_array = toml_edit::ArrayOfTables::new();
        if let Some(existing) = toml_doc
            .get("tool")
            .and_then(toml_edit::Item::as_array_of_tables)
        {
            tools_array = existing.clone();
        }

        for (tool_name, schema) in tools {
            // Determine a description from the schema if possible
            let description = schema
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("Auto-generated Rust tool");

            let mut tool_table = toml_edit::Table::new();
            tool_table.insert("name", toml_edit::value(tool_name));
            tool_table.insert("description", toml_edit::value(description));

            // For schemas, we store it as an inline table to match previous formatting,
            // but since it's arbitrary JSON, converting it via toml_edit is complex.
            // Wait, we can just store the JSON string, OR use serde_json to value mapping.
            // Actually, previously we stored it as an inline table string, but here we can just
            // convert the serde_json Value to toml_edit::Value recursively.
            // Since we don't have a direct JSON->TOML inline table converter handy without extra code,
            // we will just store it as an inline table if we map it, OR just write the raw string
            // but using a literal string in TOML so it's safe. Wait, the manifest parser expects a table.

            // Let's do a simple recursive convert if it's an object.
            // The simplest safe way is to parse the JSON string as TOML. Since JSON is mostly a subset of TOML inline tables (wait, it's not strictly).

            // The safest way is to use the `toml` crate's `serde` to convert JSON Value to `toml::Value`,
            // then format it, then parse that into `toml_edit::Item`.
            let toml_val: toml::Value = serde_json::from_value(schema.clone())
                .unwrap_or(toml::Value::Table(toml::map::Map::new()));
            let toml_str = toml::to_string(&toml_val).unwrap_or_default();
            // Parse the generated TOML string into an Item
            if let Ok(parsed_doc) = toml_str.parse::<toml_edit::DocumentMut>() {
                let table = parsed_doc.into_table();
                tool_table.insert("input_schema", toml_edit::Item::Table(table));
            }

            tools_array.push(tool_table);
        }

        toml_doc.insert("tool", toml_edit::Item::ArrayOfTables(tools_array));
    }

    let toml_content = toml_doc.to_string();

    // 7. Pack the Archive
    let out_dir = match output {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?.join("dist"),
    };

    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)?;
    }

    let out_file = out_dir.join(format!("{crate_name}.capsule"));
    pack_capsule_archive(&out_file, &toml_content, Some(&wasm_path), dir, &[])?;

    info!("🎉 Successfully built Rust capsule: {}", out_file.display());
    Ok(())
}

#[allow(dead_code)]
fn build_interactive_mcp_capsule(_dir: &Path, _output: Option<&str>) {
    // TODO: Implement the interactive dialoguer wizard for Legacy MCP
    warn!("Interactive MCP builder is currently a stub.");
}

#[allow(clippy::too_many_lines)]
fn handle_mcp_quick_convert(dir: &Path, json_filename: &str, output: Option<&str>) -> Result<()> {
    let json_path = dir.join(json_filename);
    info!(
        "🔄 Converting {} into a Universal Capsule...",
        json_path.display()
    );

    let content = fs::read_to_string(&json_path)
        .with_context(|| format!("Failed to read {}", json_path.display()))?;

    let parsed: Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON in {}", json_path.display()))?;

    // 1. Extract Package Metadata
    let name = parsed
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("legacy-mcp")
        .to_string();
    let version = parsed
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("1.0.0")
        .to_string();
    let description = parsed
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("Converted MCP capsule")
        .to_string();

    let mut toml_doc = toml_edit::DocumentMut::new();

    let mut package = toml_edit::Table::new();
    package.insert("name", toml_edit::value(name.clone()));
    package.insert("version", toml_edit::value(version));
    package.insert("description", toml_edit::value(description));
    let mut authors = toml_edit::Array::new();
    authors.push("Auto-Converter");
    package.insert("authors", toml_edit::value(authors));
    toml_doc.insert("package", toml_edit::Item::Table(package));

    let mut additional_files = Vec::new();

    // 2. Extract settings and convert to `[env]` block (gemini-extension.json specific)
    let mut env_table = toml_edit::Table::new();

    if let Some(settings) = parsed.get("settings").and_then(Value::as_array) {
        for setting in settings {
            if let Some(env_var) = setting.get("envVar").and_then(Value::as_str) {
                let req_name = setting
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(env_var);
                let desc = setting
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");

                let mut env_def = toml_edit::InlineTable::new();
                env_def.insert("type", "secret".into());
                env_def.insert("request", req_name.into());
                env_def.insert("description", desc.into());
                env_table.insert(env_var, toml_edit::value(env_def));
            }
        }
    }

    // Fallback: If no `settings` block, but we find `env` inside the mcpServers, we strip them and ask generically.
    if env_table.is_empty()
        && let Some(servers) = parsed.get("mcpServers").and_then(Value::as_object)
    {
        for (_, server_config) in servers {
            if let Some(env_map) = server_config.get("env").and_then(Value::as_object) {
                for (env_key, _) in env_map {
                    let mut env_def = toml_edit::InlineTable::new();
                    env_def.insert("type", "secret".into());
                    env_def.insert(
                        "request",
                        format!("Please provide a value for {env_key}").into(),
                    );
                    env_table.insert(env_key, toml_edit::value(env_def));
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
        let mut caps_table = toml_edit::Table::new();
        let mut host_arr = toml_edit::Array::new();
        for cap in capabilities {
            host_arr.push(cap);
        }
        caps_table.insert("host_process", toml_edit::value(host_arr));
        toml_doc.insert("capabilities", toml_edit::Item::Table(caps_table));
    }

    if !env_table.is_empty() {
        toml_doc.insert("env", toml_edit::Item::Table(env_table));
    }

    // 4. Extract MCP Servers
    let mut mcp_servers_array = toml_edit::ArrayOfTables::new();
    if let Some(servers) = parsed.get("mcpServers").and_then(Value::as_object) {
        for (server_id, server_config) in servers {
            let mut server_table = toml_edit::Table::new();
            server_table.insert("id", toml_edit::value(server_id));

            if let Some(desc) = server_config.get("description").and_then(Value::as_str) {
                server_table.insert("description", toml_edit::value(desc));
            }

            if let Some(cmd) = server_config.get("command").and_then(Value::as_str) {
                server_table.insert("type", toml_edit::value("stdio"));
                server_table.insert("command", toml_edit::value(cmd));

                if let Some(args) = server_config.get("args").and_then(Value::as_array) {
                    let mut args_arr = toml_edit::Array::new();
                    for a in args.iter().filter_map(Value::as_str) {
                        args_arr.push(a);
                    }
                    if !args_arr.is_empty() {
                        server_table.insert("args", toml_edit::value(args_arr));
                    }
                }
            } else if let Some(http_url) = server_config.get("httpUrl").and_then(Value::as_str) {
                server_table.insert("type", toml_edit::value("sse"));
                server_table.insert("url", toml_edit::value(http_url));
            }
            mcp_servers_array.push(server_table);
        }
    }

    if !mcp_servers_array.is_empty() {
        toml_doc.insert(
            "mcp_server",
            toml_edit::Item::ArrayOfTables(mcp_servers_array),
        );
    }

    // 5. Inject Context Files (AGENTS.md)
    let context_file_name = parsed
        .get("contextFileName")
        .and_then(Value::as_str)
        .unwrap_or("AGENTS.md");
    // Ensure we don't allow path traversal in the context file name
    let sanitized_context_name = Path::new(context_file_name)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let context_path = dir.join(sanitized_context_name.as_ref());

    let mut context_files_array = toml_edit::ArrayOfTables::new();
    if context_path.exists() {
        let mut ctx_table = toml_edit::Table::new();
        ctx_table.insert("name", toml_edit::value("workspace-context"));
        ctx_table.insert("file", toml_edit::value(sanitized_context_name.as_ref()));
        context_files_array.push(ctx_table);
        additional_files.push(context_path);
    }
    if !context_files_array.is_empty() {
        toml_doc.insert(
            "context_file",
            toml_edit::Item::ArrayOfTables(context_files_array),
        );
    }

    // 6. Inject Skills
    let skills_dir = dir.join("skills");
    let mut skills_array = toml_edit::ArrayOfTables::new();
    if skills_dir.exists()
        && skills_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&skills_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let skill_name = path.file_stem().unwrap_or_default().to_string_lossy();

                let mut skill_table = toml_edit::Table::new();
                skill_table.insert("name", toml_edit::value(skill_name.as_ref()));
                skill_table.insert("file", toml_edit::value(format!("skills/{file_name}")));
                skills_array.push(skill_table);
            }
        }
        additional_files.push(skills_dir);
    }
    if !skills_array.is_empty() {
        toml_doc.insert("skill", toml_edit::Item::ArrayOfTables(skills_array));
    }

    // 7. Inject Commands
    let commands_dir = dir.join("commands");
    let mut commands_array = toml_edit::ArrayOfTables::new();
    if commands_dir.exists()
        && commands_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&commands_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("toml") {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let cmd_name = format!(
                    "/{}",
                    path.file_stem().unwrap_or_default().to_string_lossy()
                );

                let mut cmd_table = toml_edit::Table::new();
                cmd_table.insert("name", toml_edit::value(cmd_name));
                cmd_table.insert("file", toml_edit::value(format!("commands/{file_name}")));
                commands_array.push(cmd_table);
            }
        }
        additional_files.push(commands_dir);
    }
    if !commands_array.is_empty() {
        toml_doc.insert("command", toml_edit::Item::ArrayOfTables(commands_array));
    }

    let toml = toml_doc.to_string();

    // 8. Pack the archive
    let out_dir = match output {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?.join("dist"),
    };

    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)?;
    }

    let out_file = out_dir.join(format!("{name}.capsule"));
    let refs: Vec<&Path> = additional_files
        .iter()
        .map(std::path::PathBuf::as_path)
        .collect();

    pack_capsule_archive(&out_file, &toml, None, dir, &refs)?;

    info!(
        "🎉 Successfully converted to universal capsule: {}",
        out_file.display()
    );
    Ok(())
}
