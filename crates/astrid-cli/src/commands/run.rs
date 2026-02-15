//! Gateway run command.

use anyhow::Result;
use astrid_gateway::{GatewayConfig, GatewayRuntime};
use colored::Colorize;

/// Start the gateway daemon.
pub(crate) async fn run_gateway(foreground: bool, config_path: Option<&str>) -> Result<()> {
    println!("{}", "Starting Astrid Gateway...".cyan().bold());

    // Load configuration: use the unified config chain, then convert to
    // GatewayConfig. An explicit path overrides the unified chain and loads
    // the gateway-specific format directly.
    let config = if let Some(path) = config_path {
        println!("  Loading config from: {}", path.yellow());
        GatewayConfig::load(path)?
    } else {
        let cwd = std::env::current_dir().ok();
        let unified = astrid_config::Config::load(cwd.as_deref())
            .map(|r| r.config)
            .unwrap_or_default();
        astrid_gateway::config_bridge::from_unified_config(&unified)
    };

    // Show configuration summary
    println!("  State directory: {}", config.gateway.state_dir.yellow());
    println!(
        "  Hot reload: {}",
        if config.gateway.hot_reload {
            "enabled".green()
        } else {
            "disabled".red()
        }
    );
    println!(
        "  Health interval: {}s",
        config.gateway.health_interval_secs.to_string().yellow()
    );

    let agent_count = config.agents.len();
    let auto_start_count = config.auto_start_agents().len();
    println!(
        "  Agents configured: {} ({} auto-start)",
        agent_count.to_string().yellow(),
        auto_start_count.to_string().green()
    );

    // Create and start runtime
    let mut runtime = GatewayRuntime::new(config)?;

    if foreground {
        println!("\n{}", "Running in foreground (Ctrl+C to stop)...".cyan());
        runtime.run().await?;
    } else {
        runtime.start().await?;

        // In a real implementation, this would daemonize
        println!(
            "\n{}",
            "Gateway started (running in foreground for now)...".green()
        );
        println!("  Press Ctrl+C to stop\n");

        runtime.run().await?;
    }

    println!("{}", "Gateway stopped.".yellow());
    Ok(())
}
