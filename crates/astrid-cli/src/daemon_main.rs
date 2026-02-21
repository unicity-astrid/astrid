//! `astridd` — standalone daemon binary for the Astrid secure agent runtime.
//!
//! This is a thin entry point that runs the daemon server directly using
//! `astrid-gateway`. It exists so that `ps` and process managers show a
//! distinct `astridd` process name.
//!
//! By default the daemon runs in **persistent** mode (stays running until
//! explicitly stopped). Pass `--ephemeral` to enable auto-shutdown when all
//! clients disconnect.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use anyhow::Result;
use clap::Parser;
use colored::Colorize;

use astrid_gateway::DaemonServer;
use astrid_gateway::server::DaemonStartOptions;

/// Astrid Daemon — background agent runtime server.
#[derive(Parser)]
#[command(name = "astridd")]
#[command(
    author,
    version,
    about = "Astrid daemon — background agent runtime server"
)]
struct Args {
    /// Run in ephemeral mode: auto-shutdown when all clients disconnect.
    #[arg(long)]
    ephemeral: bool,

    /// Override the idle-shutdown grace period (seconds).
    #[arg(long)]
    grace_period: Option<u64>,

    /// Enable verbose output.
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Set up logging.
    let level = if args.verbose { "debug" } else { "info" };
    let log_config =
        astrid_telemetry::LogConfig::new(level).with_format(astrid_telemetry::LogFormat::Compact);
    if let Err(e) = astrid_telemetry::setup_logging(&log_config) {
        eprintln!("Failed to initialize logging: {e}");
    }

    let options = DaemonStartOptions {
        ephemeral: args.ephemeral,
        grace_period_secs: args.grace_period,
        workspace_root: None,
    };

    let (daemon, handle, addr, cfg) = DaemonServer::start(options, None).await?;

    let mode_label = if daemon.is_ephemeral() {
        "ephemeral"
    } else {
        "persistent"
    };
    println!(
        "{}",
        format!("astridd listening on {addr} (mode: {mode_label})")
            .cyan()
            .bold()
    );

    // Start background tasks.
    let health_handle = daemon.spawn_health_loop();
    let cleanup_handle = daemon.spawn_session_cleanup_loop();
    let ephemeral_handle = daemon.spawn_ephemeral_monitor();
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

    println!("{}", "Daemon stopped".green().bold());
    Ok(())
}
