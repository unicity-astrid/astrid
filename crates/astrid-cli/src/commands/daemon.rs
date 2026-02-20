//! Daemon management commands (status, stop, run).

use anyhow::Result;
use astrid_gateway::DaemonServer;
use astrid_gateway::server::{DaemonPaths, DaemonStartOptions};
use colored::Colorize;

use crate::daemon_client::DaemonClient;
use crate::theme::Theme;

/// Run the daemon in the foreground (called by auto-start or `astridd`).
pub(crate) async fn run_daemon_with_mode(ephemeral: bool, grace_period: Option<u64>) -> Result<()> {
    let options = DaemonStartOptions {
        ephemeral,
        grace_period_secs: grace_period,
    };

    let (daemon, handle, addr, cfg) = DaemonServer::start(options, None).await?;

    let mode_label = if daemon.is_ephemeral() {
        "ephemeral"
    } else {
        "persistent"
    };
    println!(
        "{}",
        format!("Daemon listening on {addr} (mode: {mode_label})")
            .cyan()
            .bold()
    );

    // Start the health monitoring loop.
    let health_handle = daemon.spawn_health_loop();

    // Start session cleanup loop.
    let cleanup_handle = daemon.spawn_session_cleanup_loop();

    // Start ephemeral shutdown monitor (returns None in persistent mode).
    let ephemeral_handle = daemon.spawn_ephemeral_monitor();

    // Start plugin hot-reload watcher (gated by config, returns None if disabled
    // or no plugin dirs exist).
    let watcher_handle = if cfg.gateway.watch_plugins {
        daemon.spawn_plugin_watcher()
    } else {
        None
    };

    // Spawn embedded Telegram bot if configured.
    let telegram_handle = astrid_telegram::bot::spawn_embedded(&cfg.telegram, addr);

    // Wait for shutdown signal (Ctrl+C, RPC shutdown, or ephemeral monitor).
    let mut shutdown_rx = daemon.subscribe_shutdown();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = shutdown_rx.recv() => {},
    }

    println!("\n{}", "Shutting down daemon...".yellow());

    // Stop background tasks.
    health_handle.abort();
    cleanup_handle.abort();
    if let Some(h) = ephemeral_handle {
        h.abort();
    }
    if let Some(h) = watcher_handle {
        h.abort();
    }
    if let Some(h) = telegram_handle {
        h.abort();
    }

    // Gracefully unload all plugins before MCP shutdown.
    daemon.shutdown_plugins().await;

    // Gracefully stop all MCP servers before tearing down IPC.
    daemon.shutdown_servers().await;

    handle.stop()?;
    handle.stopped().await;
    daemon.cleanup();

    println!("{}", Theme::success("Daemon stopped"));
    Ok(())
}

/// Show daemon status.
pub(crate) async fn daemon_status() -> Result<()> {
    let paths = DaemonPaths::default_dir()?;

    if !DaemonServer::is_running(&paths) {
        println!("{}", Theme::warning("Daemon is not running"));
        return Ok(());
    }

    match DaemonClient::connect().await {
        Ok(client) => match client.status().await {
            Ok(status) => {
                println!("\n{}", Theme::header("Daemon Status"));
                println!(
                    "  Status: {}",
                    if status.running {
                        "running".green()
                    } else {
                        "stopped".red()
                    }
                );
                let mode_str = if status.ephemeral {
                    "ephemeral".yellow()
                } else {
                    "persistent".green()
                };
                println!("  Mode: {mode_str}");
                println!("  Uptime: {}s", status.uptime_secs.to_string().yellow());
                println!(
                    "  Active sessions: {}",
                    status.active_sessions.to_string().yellow()
                );
                println!(
                    "  Connections: {}",
                    status.active_connections.to_string().yellow()
                );
                println!("  Version: {}", status.version.cyan());
                println!(
                    "  MCP servers: {}/{}",
                    status.mcp_servers_running.to_string().yellow(),
                    status.mcp_servers_configured.to_string().dimmed(),
                );

                if let Some(pid) = DaemonServer::read_pid(&paths) {
                    println!("  PID: {}", pid.to_string().yellow());
                }
                if let Some(port) = DaemonServer::read_port(&paths) {
                    println!("  Port: {}", port.to_string().yellow());
                }
                println!();
            },
            Err(e) => {
                println!("{}", Theme::error(&format!("Failed to get status: {e}")));
            },
        },
        Err(e) => {
            println!(
                "{}",
                Theme::error(&format!("Failed to connect to daemon: {e}"))
            );
        },
    }

    Ok(())
}

/// Stop the daemon.
pub(crate) async fn daemon_stop() -> Result<()> {
    let paths = DaemonPaths::default_dir()?;

    if !DaemonServer::is_running(&paths) {
        println!("{}", Theme::warning("Daemon is not running"));
        return Ok(());
    }

    match DaemonClient::connect().await {
        Ok(client) => {
            client.shutdown().await?;
            println!("{}", Theme::success("Daemon shutdown requested"));
        },
        Err(e) => {
            println!(
                "{}",
                Theme::error(&format!("Failed to connect to daemon: {e}"))
            );
        },
    }

    Ok(())
}
