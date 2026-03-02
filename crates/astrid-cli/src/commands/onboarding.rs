//! First-run onboarding — guides the user through provider and API key setup.
//!
//! Supports all providers: Claude (Anthropic), Z.AI, `OpenAI`, and
//! OpenAI-compatible local servers (LM Studio, Ollama, vLLM, etc.).

use std::io::Write;
use std::path::PathBuf;

use colored::Colorize;
use dialoguer::{Input, Password, Select, theme::ColorfulTheme};

/// Known provider identifiers and their display info.
#[allow(dead_code)]
struct ProviderInfo {
    id: &'static str,
    label: &'static str,
    env_var: &'static str,
    console_url: &'static str,
    needs_api_key: bool,
    needs_api_url: bool,
}

const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "claude",
        label: "Anthropic Claude",
        env_var: "ANTHROPIC_API_KEY",
        console_url: "https://console.anthropic.com/settings/keys",
        needs_api_key: true,
        needs_api_url: false,
    },
    ProviderInfo {
        id: "zai",
        label: "Z.AI (international)",
        env_var: "ZAI_API_KEY",
        console_url: "https://z.ai",
        needs_api_key: true,
        needs_api_url: false,
    },
    ProviderInfo {
        id: "openai",
        label: "OpenAI",
        env_var: "OPENAI_API_KEY",
        console_url: "https://platform.openai.com/api-keys",
        needs_api_key: true,
        needs_api_url: false,
    },
    ProviderInfo {
        id: "openai-compat",
        label: "OpenAI-compatible (LM Studio, Ollama, vLLM, etc.)",
        env_var: "",
        console_url: "",
        needs_api_key: false, // Optional — depends on the server.
        needs_api_url: true,
    },
];

/// Run the interactive onboarding flow.
///
/// If a provider is already fully configured (API key present or local
/// endpoint set), returns immediately. Otherwise, walks the user through
/// provider selection and credential entry.
pub(crate) fn run_onboarding() {
    if is_provider_configured() {
        return;
    }

    println!();
    println!(
        "{}",
        "  Welcome to Astrid! Let's configure your LLM provider.".bold()
    );
    println!();

    // Step 1: Pick a provider.
    let labels: Vec<&str> = PROVIDERS.iter().map(|p| p.label).collect();
    let provider_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which LLM provider would you like to use?")
        .items(&labels)
        .default(0)
        .interact();

    let Ok(provider_idx) = provider_idx else {
        print_manual_instructions();
        return;
    };

    let provider = &PROVIDERS[provider_idx];
    println!();

    // Step 2: Provider-specific setup.
    match provider.id {
        "openai-compat" => setup_openai_compat(),
        _ => setup_cloud_provider(provider),
    }
}

/// Check whether the currently configured provider has what it needs.
///
/// - Cloud providers (claude, zai, openai): need an API key from env or config.
/// - `openai-compat`: needs either an API URL in config, or is accepted as-is
///   if the provider is explicitly set (user knows what they're doing).
pub(crate) fn has_api_key() -> bool {
    // Check generic override first.
    if env_is_set("ASTRID_MODEL_API_KEY") {
        return true;
    }

    // Determine configured provider from config file.
    let provider = read_config_field("provider").unwrap_or_default();

    match provider.as_str() {
        "openai-compat" => {
            // For local providers, having an api_url configured (or the
            // provider set at all) is sufficient — no key required.
            true
        },
        "zai" => env_is_set("ZAI_API_KEY") || config_has_api_key(),
        "openai" => env_is_set("OPENAI_API_KEY") || config_has_api_key(),
        // Default: Claude.
        _ => env_is_set("ANTHROPIC_API_KEY") || config_has_api_key(),
    }
}

// ---------------------------------------------------------------------------
// Provider-specific setup flows
// ---------------------------------------------------------------------------

/// Setup flow for cloud providers that require an API key (Claude, Z.AI, `OpenAI`).
fn setup_cloud_provider(provider: &ProviderInfo) {
    println!("  You'll need a {} API key.", provider.label.cyan());
    println!("  Get one at: {}", provider.console_url.cyan());
    println!();

    let options = &[
        "Enter API key now",
        "Open browser to get a key",
        "Skip (I'll set it up later)",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How would you like to proceed?")
        .items(options)
        .default(0)
        .interact();

    let Ok(selection) = selection else {
        print_manual_instructions();
        return;
    };

    match selection {
        0 => prompt_and_save_cloud(provider),
        1 => {
            if webbrowser::open(provider.console_url).is_err() {
                println!(
                    "  {}",
                    "Could not open browser. Please visit the URL above manually.".yellow()
                );
            } else {
                println!("  Browser opened. Once you have your key:");
            }
            println!();
            prompt_and_save_cloud(provider);
        },
        _ => {
            // Still save the provider choice so the user doesn't get asked again.
            let _ = save_model_config(provider.id, None, None);
            print_manual_instructions();
        },
    }
}

/// Prompt for API key and save it along with the provider to config.
fn prompt_and_save_cloud(provider: &ProviderInfo) {
    let key = Password::with_theme(&ColorfulTheme::default())
        .with_prompt("  Paste your API key")
        .interact();

    let Ok(k) = key else {
        let _ = save_model_config(provider.id, None, None);
        print_manual_instructions();
        return;
    };
    let key = k.trim().to_string();

    if key.is_empty() {
        println!("  {}", "No key entered.".yellow());
        let _ = save_model_config(provider.id, None, None);
        print_manual_instructions();
        return;
    }

    if let Err(e) = save_model_config(provider.id, Some(&key), None) {
        eprintln!("  {} {e}", "Failed to save config:".red());
        print_manual_instructions();
        return;
    }

    println!();
    println!(
        "  {} {} configured and saved to ~/.astrid/config.toml",
        "✓".green().bold(),
        provider.label
    );
    println!();
}

/// Setup flow for OpenAI-compatible / local LLM providers.
fn setup_openai_compat() {
    println!(
        "  {}",
        "OpenAI-compatible providers work with LM Studio, Ollama, vLLM, and more.".dimmed()
    );
    println!();

    // Ask for API URL.
    let default_url = "http://localhost:1234/v1/chat/completions";
    let url = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("  API endpoint URL")
        .default(default_url.to_string())
        .interact_text();

    let Ok(u) = url else {
        let _ = save_model_config("openai-compat", None, Some(default_url));
        print_manual_instructions();
        return;
    };
    let url = u.trim().to_string();

    let is_local = is_local_url(&url);

    // Ask for API key (optional for local servers).
    let api_key = if is_local {
        let needs_key = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Does your local server require an API key?")
            .items(&["No (typical for LM Studio, Ollama)", "Yes"])
            .default(0)
            .interact();

        match needs_key {
            Ok(1) => prompt_optional_key(),
            _ => None,
        }
    } else {
        println!("  Remote endpoint detected — an API key is likely required.");
        prompt_optional_key()
    };

    // Ask for model name.
    let default_model = if is_local { "local-model" } else { "gpt-4o" };
    let model = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("  Model name")
        .default(default_model.to_string())
        .interact_text()
        .unwrap_or_else(|_| default_model.to_string());

    // Ask for context window size.
    let default_ctx: usize = 32_768;
    println!();
    println!(
        "  {}",
        "Set the context window size (in tokens) for your model.".dimmed()
    );
    println!(
        "  {}",
        "Common sizes: 4096, 8192, 32768, 65536, 128000, 131072".dimmed()
    );
    let context_window: usize = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("  Context window (tokens)")
        .default(default_ctx)
        .interact_text()
        .unwrap_or(default_ctx);

    if let Err(e) = save_model_config_full(
        "openai-compat",
        api_key.as_deref(),
        Some(&url),
        Some(&model),
        Some(context_window),
    ) {
        eprintln!("  {} {e}", "Failed to save config:".red());
        print_manual_instructions();
        return;
    }

    println!();
    println!(
        "  {} OpenAI-compatible provider configured ({})",
        "✓".green().bold(),
        url.dimmed()
    );
    println!();
}

/// Prompt for an optional API key. Returns `None` if empty/cancelled.
fn prompt_optional_key() -> Option<String> {
    let key = Password::with_theme(&ColorfulTheme::default())
        .with_prompt("  Paste your API key (or press Enter to skip)")
        .allow_empty_password(true)
        .interact()
        .ok()?;

    let key = key.trim().to_string();
    if key.is_empty() { None } else { Some(key) }
}

// ---------------------------------------------------------------------------
// Config file helpers
// ---------------------------------------------------------------------------

/// Save provider, optional `api_key`, and optional `api_url` to the `[model]`
/// section of `~/.astrid/config.toml`.
fn save_model_config(
    provider: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
) -> std::io::Result<()> {
    save_model_config_full(provider, api_key, api_url, None, None)
}

/// Save provider, optional `api_key`, optional `api_url`, optional model name,
/// and optional context window to the `[model]` section of
/// `~/.astrid/config.toml`.
fn save_model_config_full(
    provider: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
    model: Option<&str>,
    context_window: Option<usize>,
) -> std::io::Result<()> {
    let config_path = global_config_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "Cannot resolve config path")
    })?;

    let contents = std::fs::read_to_string(&config_path).unwrap_or_default();

    // Build the lines to insert after [model].
    let mut new_lines = Vec::new();
    new_lines.push(format!("provider = \"{provider}\""));
    if let Some(key) = api_key {
        new_lines.push(format!("api_key = \"{key}\""));
    }
    if let Some(url) = api_url {
        new_lines.push(format!("api_url = \"{url}\""));
    }
    if let Some(m) = model {
        new_lines.push(format!("model = \"{m}\""));
    }
    if let Some(ctx) = context_window {
        new_lines.push(format!("context_window = {ctx}"));
    }
    let insert_block = new_lines.join("\n");

    let new_contents = if contents.contains("[model]") {
        // Replace the [model] section header and insert our lines after it,
        // removing any existing commented/uncommented lines for fields we set.
        let fields_to_replace: Vec<&str> = {
            let mut f = vec!["provider"];
            if api_key.is_some() {
                f.push("api_key");
            }
            if api_url.is_some() {
                f.push("api_url");
            }
            if model.is_some() {
                f.push("model");
            }
            if context_window.is_some() {
                f.push("context_window");
            }
            f
        };

        let mut result = String::new();
        let mut inserted = false;
        for line in contents.lines() {
            let trimmed = line.trim();

            // Skip existing lines (commented or not) for fields we're setting.
            let skip = fields_to_replace.iter().any(|field| {
                let uncommented = trimmed.starts_with(field) && trimmed.contains('=');
                let commented = trimmed.starts_with('#')
                    && trimmed[1..].trim_start().starts_with(field)
                    && trimmed.contains('=');
                uncommented || commented
            });

            if skip {
                continue;
            }

            result.push_str(line);
            result.push('\n');

            if !inserted && trimmed == "[model]" {
                result.push_str(&insert_block);
                result.push('\n');
                inserted = true;
            }
        }
        result
    } else {
        // Append a new [model] section.
        let mut result = contents;
        if !result.ends_with('\n') && !result.is_empty() {
            result.push('\n');
        }
        result.push_str("\n[model]\n");
        result.push_str(&insert_block);
        result.push('\n');
        result
    };

    let mut f = std::fs::File::create(&config_path)?;
    f.write_all(new_contents.as_bytes())?;

    Ok(())
}

/// Read a field from the `[model]` section of the config file.
///
/// Returns the raw string value (without quotes), or `None` if not found.
fn read_config_field(field: &str) -> Option<String> {
    let path = global_config_path()?;
    let contents = std::fs::read_to_string(path).ok()?;

    let mut in_model_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();

        // Track sections.
        if trimmed.starts_with('[') {
            in_model_section = trimmed == "[model]";
            continue;
        }

        if !in_model_section || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with(field)
            && trimmed.contains('=')
            && let Some(val) = trimmed.split('=').nth(1)
        {
            let val = val.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }

    None
}

/// Check if config has a non-empty `api_key` in the `[model]` section.
fn config_has_api_key() -> bool {
    read_config_field("api_key").is_some()
}

/// Check if an environment variable is set and non-empty.
fn env_is_set(var: &str) -> bool {
    std::env::var(var).ok().is_some_and(|v| !v.is_empty())
}

/// Check whether a URL points to a local endpoint.
fn is_local_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("localhost") || lower.contains("127.0.0.1") || lower.contains("[::1]")
}

/// Check whether the provider is fully configured and ready to use.
///
/// This goes beyond `has_api_key()` — it also checks whether the config
/// has any provider set at all (even `openai-compat` with no key).
fn is_provider_configured() -> bool {
    // If any API key env var is set, we're good.
    if env_is_set("ASTRID_MODEL_API_KEY")
        || env_is_set("ANTHROPIC_API_KEY")
        || env_is_set("ZAI_API_KEY")
    {
        return true;
    }

    // If the config file has an api_key, we're good.
    if config_has_api_key() {
        return true;
    }

    // If a provider is explicitly set in config, assume the user configured it.
    // (Handles openai-compat with local server and no key.)
    read_config_field("provider").is_some()
}

/// Print manual instructions for setting up any provider.
fn print_manual_instructions() {
    println!();
    println!("  {}", "To configure your provider later:".bold());
    println!();
    println!("  Edit {}:", "~/.astrid/config.toml".cyan());
    println!();
    println!("    # For Anthropic Claude:");
    println!("    [model]");
    println!("    provider = \"claude\"");
    println!("    api_key = \"sk-ant-...\"");
    println!();
    println!("    # For Z.AI:");
    println!("    [model]");
    println!("    provider = \"zai\"");
    println!("    api_key = \"...\"");
    println!();
    println!("    # For OpenAI:");
    println!("    [model]");
    println!("    provider = \"openai\"");
    println!("    api_key = \"sk-...\"");
    println!();
    println!("    # For a local LLM (LM Studio, Ollama, etc.):");
    println!("    [model]");
    println!("    provider = \"openai-compat\"");
    println!("    api_url = \"http://localhost:1234/v1/chat/completions\"");
    println!("    model = \"local-model\"");
    println!("    context_window = 32768");
    println!();
    println!(
        "  Or set an env var: {}, {}, or {}",
        "ANTHROPIC_API_KEY".cyan(),
        "ZAI_API_KEY".cyan(),
        "OPENAI_API_KEY".cyan(),
    );
    println!();
}

/// Resolve the path to `~/.astrid/config.toml`.
fn global_config_path() -> Option<PathBuf> {
    astrid_core::dirs::AstridHome::resolve()
        .ok()
        .map(|h| h.config_path())
}

// ---------------------------------------------------------------------------
// Spark (identity) onboarding
// ---------------------------------------------------------------------------

/// Run the spark identity onboarding flow.
///
/// Checks if `~/.astrid/spark.toml` already exists — if so, skips.
/// Otherwise, offers the user a chance to name their agent.
pub(crate) fn run_spark_onboarding() {
    let spark_path = match astrid_core::dirs::AstridHome::resolve() {
        Ok(home) => home.spark_path(),
        Err(_) => return,
    };

    // If spark.toml already exists with content, skip.
    if spark_path.exists()
        && let Ok(contents) = std::fs::read_to_string(&spark_path)
        && !contents.trim().is_empty()
    {
        return;
    }

    println!();
    let should_name = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Would you like to name your agent?")
        .items(&["Yes", "Skip (agent starts without a name)"])
        .default(0)
        .interact();

    let Ok(selection) = should_name else {
        return;
    };

    if selection != 0 {
        return;
    }

    let callsign = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("  Agent name")
        .default("Stellar".to_string())
        .interact_text()
        .unwrap_or_else(|_| "Stellar".to_string());

    let class = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("  Role (optional, e.g. navigator, engineer)")
        .allow_empty(true)
        .interact_text()
        .unwrap_or_default();

    let aura = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("  Personality (optional, e.g. calm, sharp, warm)")
        .allow_empty(true)
        .interact_text()
        .unwrap_or_default();

    let signal = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("  Communication style (optional, e.g. concise, formal)")
        .allow_empty(true)
        .interact_text()
        .unwrap_or_default();

    // Build and write spark.toml using proper TOML serialization (avoids injection)
    let spark = astrid_tools::spark::SparkConfig {
        callsign: callsign.trim().to_string(),
        class: class.trim().to_string(),
        aura: aura.trim().to_string(),
        signal: signal.trim().to_string(),
        core: String::new(),
    };

    match spark.save_to_file(&spark_path) {
        Ok(()) => {
            println!(
                "\n  {} Agent identity saved to {}",
                "✓".green().bold(),
                "~/.astrid/spark.toml".cyan()
            );
            println!();
        },
        Err(e) => {
            eprintln!("  {} Failed to save spark: {e}", "✗".red().bold());
        },
    }
}
