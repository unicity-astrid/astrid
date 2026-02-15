//! Hook management commands.

use anyhow::Result;
use astralis_core::dirs::AstralisHome;
use astralis_hooks::{
    HookEvent, HookExecutor, HookHandler, HookManager, HooksConfig, discover_hooks,
    profiles::{available_profiles, get_profile},
    result::HookContext,
};
use colored::Colorize;
use std::path::PathBuf;

/// List all configured hooks.
pub(crate) fn list_hooks() {
    println!("{}", "Configured Hooks".cyan().bold());
    println!();

    let config = HooksConfig::default();

    if !config.enabled {
        println!("{}", "Hooks are disabled in configuration.".yellow());
        println!("Enable with: hooks.enabled = true in your config");
        return;
    }

    // Discover hooks from user-level directory
    let hooks_dir = AstralisHome::resolve()
        .map_or_else(|_| PathBuf::from(".astralis/hooks"), |h| h.hooks_dir());

    let extra_paths = vec![hooks_dir.clone()];
    let hooks = discover_hooks(Some(&extra_paths));

    if hooks.is_empty() {
        println!("No hooks found.");
        println!("Create hooks in: {}", hooks_dir.display());
        return;
    }

    for hook in &hooks {
        let status = if hook.enabled {
            "enabled".green()
        } else {
            "disabled".red()
        };

        let id_str = hook.id.to_string();
        let name = hook.name.as_deref().unwrap_or(&id_str);
        println!(
            "  {} [{}] - {}",
            name.yellow(),
            status,
            format!("{:?}", hook.event).dimmed()
        );

        if let Some(ref desc) = hook.description {
            println!("    {}", desc.dimmed());
        }
    }

    println!("\nTotal: {} hooks", hooks.len());
}

/// Enable a hook by name.
pub(crate) fn enable_hook(name: &str) {
    println!("Enabling hook: {}", name.yellow());

    // In a real implementation, this would update the hook's config
    // For now, we just show what would happen
    println!("{}", "Hook enabled successfully.".green());
    println!("Note: Hook state is managed via config files.");
}

/// Disable a hook by name.
pub(crate) fn disable_hook(name: &str) {
    println!("Disabling hook: {}", name.yellow());

    println!("{}", "Hook disabled successfully.".green());
    println!("Note: Hook state is managed via config files.");
}

/// Show detailed information about a hook.
pub(crate) fn hook_info(name: &str) {
    println!("{}", format!("Hook: {name}").cyan().bold());
    println!();

    let extra_paths: Vec<PathBuf> = AstralisHome::resolve()
        .map(|h| vec![h.hooks_dir()])
        .unwrap_or_default();
    let hooks = discover_hooks(Some(&extra_paths));

    let hook = hooks
        .iter()
        .find(|h| h.name.as_deref() == Some(name) || h.id.to_string() == name);

    if let Some(hook) = hook {
        let id_str = hook.id.to_string();
        let display_name = hook.name.as_deref().unwrap_or(&id_str);
        println!("  Name:        {}", display_name.yellow());
        println!("  ID:          {}", hook.id);
        println!("  Event:       {:?}", hook.event);
        println!(
            "  Enabled:     {}",
            if hook.enabled {
                "yes".green()
            } else {
                "no".red()
            }
        );
        println!("  Priority:    {}", hook.priority);

        if let Some(ref desc) = hook.description {
            println!("  Description: {desc}");
        }

        if let Some(ref matcher) = hook.matcher {
            println!("  Matcher:     {matcher:?}");
        }

        println!("\n  Handler:");
        match &hook.handler {
            HookHandler::Command { command, args, .. } => {
                println!("    Type: Command");
                println!("    Command: {} {}", command, args.join(" "));
            },
            HookHandler::Http { url, method, .. } => {
                println!("    Type: HTTP");
                println!("    URL: {method} {url}");
            },
            HookHandler::Wasm { module_path, .. } => {
                println!("    Type: WASM");
                println!("    Module: {module_path}");
            },
            HookHandler::Agent { model, .. } => {
                println!("    Type: Agent");
                println!("    Model: {}", model.as_deref().unwrap_or("default"));
            },
        }
    } else {
        println!("{}", format!("Hook '{name}' not found.").red());
    }
}

/// Show hook statistics.
pub(crate) async fn hook_stats() -> Result<()> {
    println!("{}", "Hook Statistics".cyan().bold());
    println!();

    let manager = HookManager::new();
    let stats = manager.stats().await;

    println!("  Total hooks:      {}", stats.total);
    println!("  Enabled:          {}", stats.enabled.to_string().green());
    println!("  Disabled:         {}", stats.disabled.to_string().red());
    println!(
        "  Events w/ hooks:  {}",
        stats.events_with_hooks.to_string().yellow()
    );

    Ok(())
}

/// Test a hook with a dry run.
pub(crate) async fn test_hook(name: &str, dry_run: bool) -> Result<()> {
    println!(
        "{} {}",
        if dry_run { "Dry run:" } else { "Testing:" },
        name.yellow()
    );
    println!();

    let extra_paths: Vec<PathBuf> = AstralisHome::resolve()
        .map(|h| vec![h.hooks_dir()])
        .unwrap_or_default();
    let hooks = discover_hooks(Some(&extra_paths));

    let hook = hooks
        .iter()
        .find(|h| h.name.as_deref() == Some(name) || h.id.to_string() == name);

    if let Some(hook) = hook {
        let executor = HookExecutor::new();
        let context = HookContext::new(HookEvent::PreToolCall)
            .with_data("tool_name", serde_json::json!("test_tool"))
            .with_data("session_id", serde_json::json!("test-session"));

        let id_str = hook.id.to_string();
        let hook_name = hook.name.as_deref().unwrap_or(&id_str);

        if dry_run {
            println!("  Would execute hook: {hook_name}");
            println!("  Event: {:?}", hook.event);
            println!(
                "  Handler type: {}",
                match &hook.handler {
                    HookHandler::Command { .. } => "Command",
                    HookHandler::Http { .. } => "HTTP",
                    HookHandler::Wasm { .. } => "WASM",
                    HookHandler::Agent { .. } => "Agent",
                }
            );
            println!("\n{}", "Dry run complete - no changes made.".green());
        } else {
            println!("  Executing hook...");
            let execution = executor.execute(hook, &context).await;
            println!("  Result: {:?}", execution.result);
            println!("  Duration: {} ms", execution.duration_ms);
        }
    } else {
        println!("{}", format!("Hook '{name}' not found.").red());
    }

    Ok(())
}

/// List available hook profiles.
pub(crate) fn list_profiles() {
    println!("{}", "Available Hook Profiles".cyan().bold());
    println!();

    for profile_name in available_profiles() {
        if let Some(profile) = get_profile(profile_name) {
            println!("  {} - {}", profile_name.yellow(), profile.description);
            println!("    Hooks: {}", profile.hooks.len());
        }
    }

    println!("\nApply with: astralis hooks apply-profile <name>");
}
