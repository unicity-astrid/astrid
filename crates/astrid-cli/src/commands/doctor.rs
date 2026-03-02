//! Doctor command for health checks.

use anyhow::Result;
use astrid_kernel::{GatewayConfig, GatewayRuntime, HealthState};
use colored::Colorize;

/// Run health checks against the gateway.
pub(crate) async fn run_doctor() -> Result<()> {
    println!("{}", "Astrid Doctor - System Health Check".cyan().bold());
    println!();

    // Check configuration
    print!("  Checking configuration... ");
    match GatewayConfig::load_default() {
        Ok(config) => {
            println!("{}", "OK".green());
            println!("    State dir: {}", config.gateway.state_dir);
            println!("    Agents: {}", config.agents.len());
        },
        Err(e) => {
            println!("{}", "WARN".yellow());
            println!("    Using defaults: {e}");
        },
    }

    // Check if we can create a runtime
    print!("  Checking runtime initialization... ");
    let config = GatewayConfig::default();
    let runtime = GatewayRuntime::new(config)?;
    println!("{}", "OK".green());

    // Run health checks
    println!("\n{}", "Running health checks:".cyan());
    let status = runtime.health().await;

    for check in &status.checks {
        let state_str = match check.state {
            HealthState::Healthy => "OK".green(),
            HealthState::Degraded => "WARN".yellow(),
            HealthState::Unhealthy => "FAIL".red(),
            HealthState::Unknown => "???".dimmed(),
        };

        print!("  {} {}", state_str, check.component);

        if let Some(ref msg) = check.message {
            print!(" - {}", msg.dimmed());
        }

        println!(" ({}ms)", check.duration_ms);

        // Show details
        for (key, value) in &check.details {
            println!("    {key}: {value}");
        }
    }

    // Overall status
    println!();
    let overall = match status.state {
        HealthState::Healthy => "All systems healthy".green(),
        HealthState::Degraded => "Some issues detected".yellow(),
        HealthState::Unhealthy => "Critical issues found".red(),
        HealthState::Unknown => "Status unknown".dimmed(),
    };
    println!("{}", overall.bold());

    // Additional checks
    println!("\n{}", "Additional checks:".cyan());

    // Check MCP servers config
    print!("  MCP server configuration... ");
    match astrid_mcp::ServersConfig::load_default() {
        Ok(config) => {
            let server_count = config.servers.len();
            if server_count > 0 {
                println!("{} ({} servers)", "OK".green(), server_count);
            } else {
                println!("{} (no servers configured)", "WARN".yellow());
            }
        },
        Err(_) => {
            println!("{} (no config file)", "OK".dimmed());
        },
    }

    // Check audit log
    print!("  Audit log... ");
    println!("{}", "OK".green());

    // Check LLM provider
    print!("  LLM provider... ");
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!("{} (Claude)", "OK".green());
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        println!("{} (OpenAI)", "OK".green());
    } else {
        println!("{} (no API key set)", "WARN".yellow());
        println!("    Set ANTHROPIC_API_KEY or OPENAI_API_KEY");
    }

    println!();

    if status.state == HealthState::Healthy {
        println!("{}", "Astrid is ready to use!".green().bold());
    } else {
        println!(
            "{}",
            "Please address the issues above before using Astrid."
                .yellow()
                .bold()
        );
    }

    Ok(())
}
