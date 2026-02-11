//! Servers command - manage MCP servers.

use astralis_gateway::rpc::ToolInfo;
use astralis_mcp::ServersConfig;
use colored::Colorize;

use crate::daemon_client::DaemonClient;
use crate::theme::Theme;

/// List MCP servers via the daemon RPC (live status).
pub(crate) async fn list_servers_via_daemon(client: &DaemonClient) -> anyhow::Result<()> {
    let servers = client.list_servers().await?;

    if servers.is_empty() {
        println!("{}", Theme::info("No MCP servers running in daemon"));
        println!(
            "{}",
            Theme::dimmed("Add servers to ~/.astralis/servers.toml")
        );
        return Ok(());
    }

    println!("\n{}", Theme::header("MCP Servers (via daemon)"));
    println!(
        "{:>15} {:>10} {:>8} {:>10} {}",
        "NAME".dimmed(),
        "STATUS".dimmed(),
        "TOOLS".dimmed(),
        "RESTARTS".dimmed(),
        "DESCRIPTION".dimmed(),
    );
    println!("{}", Theme::separator());

    for server in &servers {
        let status = if server.ready {
            "online".green()
        } else if server.alive {
            "connecting".yellow()
        } else {
            "offline".red()
        };

        let desc = server.description.as_deref().unwrap_or("-");

        println!(
            "{:>15} {:>10} {:>8} {:>10} {}",
            server.name.cyan(),
            status,
            server.tool_count.to_string().yellow(),
            server.restart_count.to_string().dimmed(),
            desc.dimmed(),
        );
    }

    println!();
    Ok(())
}

/// List all configured servers (fallback when daemon is not running).
pub(crate) fn list_servers(config: &ServersConfig) {
    let servers = config.list();

    if servers.is_empty() {
        println!("{}", Theme::info("No servers configured"));
        println!(
            "{}",
            Theme::dimmed("Add servers to ~/.astralis/servers.toml")
        );
        return;
    }

    println!("\n{}", Theme::header("Configured Servers"));
    println!(
        "{:>15} {:>10} {:>12} {}",
        "NAME".dimmed(),
        "TRANSPORT".dimmed(),
        "AUTO-START".dimmed(),
        "COMMAND".dimmed()
    );
    println!("{}", Theme::separator());

    for name in servers {
        if let Some(server) = config.get(name) {
            let transport = format!("{:?}", server.transport).to_lowercase();
            let auto_start = if server.auto_start {
                "yes".green()
            } else {
                "no".dimmed()
            };
            let command = server
                .command
                .as_deref()
                .or(server.url.as_deref())
                .unwrap_or("-");

            println!(
                "{:>15} {:>10} {:>12} {}",
                name.cyan(),
                transport,
                auto_start,
                command.dimmed()
            );
        }
    }

    println!();
}

/// Start a server via the daemon.
pub(crate) async fn start_server(client: &DaemonClient, name: &str) -> anyhow::Result<()> {
    println!("{}", Theme::info(&format!("Starting {name}...")));
    client.start_server(name).await?;
    println!("{}", Theme::success(&format!("Started {name}")));
    Ok(())
}

/// Stop a server via the daemon.
pub(crate) async fn stop_server(client: &DaemonClient, name: &str) -> anyhow::Result<()> {
    println!("{}", Theme::info(&format!("Stopping {name}...")));
    client.stop_server(name).await?;
    println!("{}", Theme::success(&format!("Stopped {name}")));
    Ok(())
}

/// List available tools via the daemon.
pub(crate) async fn list_tools(client: &DaemonClient) -> anyhow::Result<()> {
    let tools: Vec<ToolInfo> = client.list_tools().await?;

    if tools.is_empty() {
        println!("{}", Theme::info("No tools available"));
        println!("{}", Theme::dimmed("Start some servers first"));
        return Ok(());
    }

    println!("\n{}", Theme::header("Available Tools"));
    println!(
        "{:>20} {:>15} {}",
        "TOOL".dimmed(),
        "SERVER".dimmed(),
        "DESCRIPTION".dimmed()
    );
    println!("{}", Theme::separator());

    for tool in tools {
        let desc = tool.description.as_deref().map_or_else(
            || "-".to_string(),
            |d| {
                if d.len() > 40 {
                    format!("{}...", &d[..40])
                } else {
                    d.to_string()
                }
            },
        );

        println!(
            "{:>20} {:>15} {}",
            tool.name.cyan(),
            tool.server.dimmed(),
            desc.dimmed()
        );
    }

    println!();
    Ok(())
}
