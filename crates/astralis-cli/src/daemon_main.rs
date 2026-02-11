#![deny(unsafe_code)]
#![deny(clippy::all)]
//! `astralisd` — standalone daemon binary for the Astralis secure agent runtime.
//!
//! This is a thin entry point that runs the daemon server directly using
//! `astralis-gateway`. It exists so that `ps` and process managers show a
//! distinct `astralisd` process name.
//!
//! By default the daemon runs in **persistent** mode (stays running until
//! explicitly stopped). Pass `--ephemeral` to enable auto-shutdown when all
//! clients disconnect.

use anyhow::Result;
use clap::Parser;
use colored::Colorize;

use astralis_gateway::DaemonServer;
use astralis_gateway::server::DaemonStartOptions;

/// Astralis Daemon — background agent runtime server.
#[derive(Parser)]
#[command(name = "astralisd")]
#[command(
    author,
    version,
    about = "Astralis daemon — background agent runtime server"
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
    let log_config = astralis_telemetry::LogConfig::new(level)
        .with_format(astralis_telemetry::LogFormat::Compact);
    if let Err(e) = astralis_telemetry::setup_logging(&log_config) {
        eprintln!("Failed to initialize logging: {e}");
    }

    let options = DaemonStartOptions {
        ephemeral: args.ephemeral,
        grace_period_secs: args.grace_period,
    };

    let (daemon, handle, addr) = DaemonServer::start(options).await?;

    let mode_label = if daemon.is_ephemeral() {
        "ephemeral"
    } else {
        "persistent"
    };
    println!(
        "{}",
        format!("astralisd listening on {addr} (mode: {mode_label})")
            .cyan()
            .bold()
    );

    // Start background tasks.
    let health_handle = daemon.spawn_health_loop();
    let cleanup_handle = daemon.spawn_session_cleanup_loop();
    let ephemeral_handle = daemon.spawn_ephemeral_monitor();

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

    daemon.shutdown_servers().await;

    handle.stop()?;
    handle.stopped().await;
    daemon.cleanup();

    println!("{}", "Daemon stopped".green().bold());
    Ok(())
}
