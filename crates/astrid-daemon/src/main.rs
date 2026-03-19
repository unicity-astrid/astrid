//! Astrid Daemon - Background kernel process for the Astrid secure agent runtime.
//!
//! This binary is the daemon that CLI and other frontends connect to via Unix
//! socket. It boots the kernel, loads capsules, and serves IPC requests.
//!
//! Typically spawned by the CLI (`astrid chat`), but can also be started
//! directly for persistent multi-frontend operation (`astrid start`).

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]

use anyhow::{Context, Result};
use clap::Parser;

/// Astrid Daemon - Background kernel process
#[derive(Parser)]
#[command(name = "astrid-daemon")]
#[command(author, version, about)]
struct Args {
    /// The session ID to bind the daemon to
    #[arg(short, long, default_value = "00000000-0000-0000-0000-000000000000")]
    session: String,

    /// Workspace root directory
    #[arg(short, long)]
    workspace: Option<std::path::PathBuf>,

    /// Enable ephemeral mode (auto-shutdown on idle timeout after last client disconnects)
    #[arg(long)]
    ephemeral: bool,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

fn init_logging(verbose: bool) {
    let workspace_root = std::env::current_dir().ok();
    let unified_cfg = astrid_config::Config::load(workspace_root.as_deref())
        .ok()
        .map(|r| r.config);

    let log_config = if let Some(cfg) = &unified_cfg {
        let mut lc = astrid_telemetry::log_config_from(cfg);
        if verbose {
            "debug".clone_into(&mut lc.level);
        }
        if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            lc.target = astrid_telemetry::LogTarget::File(home.log_dir());
        }
        lc
    } else {
        let level = if verbose { "debug" } else { "info" };
        let mut lc = astrid_telemetry::LogConfig::new(level)
            .with_format(astrid_telemetry::LogFormat::Compact);
        if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            lc.target = astrid_telemetry::LogTarget::File(home.log_dir());
        }
        lc
    };

    if let Err(e) = astrid_telemetry::setup_logging(&log_config) {
        eprintln!("Failed to initialize logging: {e}");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    init_logging(args.verbose);

    let session_id = astrid_core::SessionId::from_uuid(
        uuid::Uuid::parse_str(&args.session)
            .map_err(|e| anyhow::anyhow!("Invalid UUID format: {e}"))?,
    );

    let ws = args.workspace.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });

    let kernel = astrid_kernel::Kernel::new(session_id.clone(), ws)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to boot Kernel: {e}"))?;

    // In ephemeral mode, shut down immediately when the last client disconnects.
    if args.ephemeral {
        kernel.set_ephemeral(true);
    }

    // Load all capsules (auto-discovery)
    kernel.load_all_capsules().await;

    // Verify the CLI proxy capsule loaded. Without it, the daemon
    // has no accept loop and CLI connections will always time out.
    {
        let reg = kernel.capsules.read().await;
        let has_cli_proxy = reg
            .list()
            .iter()
            .any(|id| id.as_str() == "astrid-capsule-cli");
        if !has_cli_proxy {
            tracing::error!(
                "CLI proxy capsule (astrid-capsule-cli) not found - \
                 daemon cannot accept CLI connections"
            );
            anyhow::bail!(
                "CLI proxy capsule (astrid-capsule-cli) not found. \
                 Install it with: astrid capsule install @unicity-astrid/capsule-cli"
            );
        }
    }

    // Signal readiness AFTER all capsules are loaded and accepting
    // connections. The CLI polls for this file to avoid connecting
    // before the handshake accept loop is running.
    astrid_kernel::socket::write_readiness_file().map_err(|e| {
        anyhow::anyhow!(
            "Failed to write readiness file \
             (daemon is useless without it): {e}"
        )
    })?;

    tracing::info!(
        session = %session_id.0,
        ephemeral = args.ephemeral,
        "Kernel booted successfully"
    );

    // Wait for a termination signal or API shutdown request.
    let mut shutdown_rx = kernel.shutdown_tx.subscribe();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("failed to register SIGTERM handler")?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received SIGINT, shutting down");
            }
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM, shutting down");
            }
            _ = shutdown_rx.wait_for(|v| *v) => {
                tracing::info!("Received API shutdown request, shutting down");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received SIGINT, shutting down");
            }
            _ = shutdown_rx.wait_for(|v| *v) => {
                tracing::info!("Received API shutdown request, shutting down");
            }
        }
    }

    kernel.shutdown(Some("signal".to_string())).await;

    Ok(())
}
