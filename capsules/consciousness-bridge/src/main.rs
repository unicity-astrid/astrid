//! Consciousness Bridge MCP Server
//!
//! Native binary that bridges minime's `WebSocket` telemetry with Astrid's
//! IPC bus. Launched as an MCP stdio subprocess by the Astrid kernel.
//!
//! Responsibilities:
//! - Subscribe to minime's spectral telemetry on <ws://127.0.0.1:7878>
//! - Send sensory input to minime on <ws://127.0.0.1:7879>
//! - Log all bridged messages to `SQLite`
//! - Expose MCP tools for the WASM component to call
//! - Enforce spectral safety protocol

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use consciousness_bridge_server::{
    autonomous, condition_metrics,
    db::BridgeDb,
    mcp,
    paths::{BridgePathOverrides, configure_bridge_paths},
    ws,
};
use tokio::sync::{RwLock, mpsc};
use tracing::{info, warn};

use ws::BridgeState;

/// Consciousness bridge MCP server.
#[derive(Parser)]
#[command(name = "consciousness-bridge-server", version)]
struct Cli {
    /// Minime telemetry `WebSocket` address (outbound eigenvalue stream).
    #[arg(long, default_value = "ws://127.0.0.1:7878")]
    minime_telemetry: String,

    /// Minime sensory input `WebSocket` address (inbound features).
    #[arg(long, default_value = "ws://127.0.0.1:7879")]
    minime_sensory: String,

    /// Path to the `SQLite` database file.
    #[arg(long, default_value = "consciousness_bridge.db")]
    db_path: String,

    /// Message retention in seconds (default: 90 days — keep everything, disk is plentiful).
    #[arg(long, default_value_t = 7_776_000)]
    retention_secs: u64,

    /// Enable autonomous feedback loop (Astrid responds to minime's spectral
    /// state without manual stimulus).
    #[arg(long)]
    autonomous: bool,

    /// Interval in seconds between autonomous exchanges (default: 20).
    #[arg(long, default_value_t = 20)]
    auto_interval_secs: u64,

    /// Reservoir sandbox `WebSocket` address used by autonomous reservoir actions.
    #[arg(long, env = "RESERVOIR_WS_URL", default_value = "ws://127.0.0.1:7881")]
    reservoir_ws_url: String,

    /// Path to minime's workspace directory (for reading journal entries
    /// during autonomous mode).
    #[arg(long, env = "MINIME_WORKSPACE")]
    workspace_path: Option<PathBuf>,

    /// Path to Astrid's perception directory (visual/audio input from the
    /// perception capsule).
    #[arg(long, env = "ASTRID_PERCEPTION_PATH")]
    perception_path: Option<PathBuf>,

    /// Path to the bridge checkout root.
    #[arg(long, env = "ASTRID_BRIDGE_ROOT")]
    bridge_root: Option<PathBuf>,

    /// Path to the bridge workspace directory for runtime artifacts.
    #[arg(long, env = "ASTRID_BRIDGE_WORKSPACE")]
    bridge_workspace: Option<PathBuf>,

    /// Path to the Astrid repo root.
    #[arg(long, env = "ASTRID_ROOT")]
    astrid_root: Option<PathBuf>,

    /// Path to the minime repo root.
    #[arg(long, env = "MINIME_ROOT")]
    minime_root: Option<PathBuf>,

    /// Path to the introspector MCP helper script.
    #[arg(long, env = "ASTRID_INTROSPECTOR_SCRIPT")]
    introspector_script: Option<PathBuf>,

    /// Path to the reflective MLX sidecar script.
    #[arg(long, env = "ASTRID_REFLECTIVE_SIDECAR")]
    reflective_sidecar_script: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    autonomous::configure_reservoir_service(Some(cli.reservoir_ws_url.clone()));
    let resolved_paths = configure_bridge_paths(BridgePathOverrides {
        bridge_root: cli.bridge_root.clone(),
        bridge_workspace: cli.bridge_workspace.clone(),
        astrid_root: cli.astrid_root.clone(),
        autoresearch_root: None,
        minime_root: cli.minime_root.clone(),
        minime_workspace: cli.workspace_path.clone(),
        perception_path: cli.perception_path.clone(),
        introspector_script: cli.introspector_script.clone(),
        reflective_sidecar_script: cli.reflective_sidecar_script.clone(),
    });

    for (label, result) in [
        (
            "Astrid journal",
            consciousness_bridge_server::managed_dir::compact_text_directory(
                &resolved_paths.astrid_journal_dir(),
            ),
        ),
        (
            "Astrid perceptions",
            consciousness_bridge_server::managed_dir::compact_json_directory(
                resolved_paths.perception_path(),
            ),
        ),
    ] {
        match result {
            Ok(created) if !created.is_empty() => {
                info!(
                    label = label,
                    buckets = created.len(),
                    "compacted managed directory"
                );
            },
            Ok(_) => {},
            Err(error) => {
                warn!(label = label, error = %error, "managed directory compaction failed");
            },
        }
    }

    info!(
        telemetry = %cli.minime_telemetry,
        sensory = %cli.minime_sensory,
        db = %cli.db_path,
        bridge_workspace = %resolved_paths.bridge_workspace().display(),
        minime_workspace = %resolved_paths.minime_workspace().display(),
        perception = %resolved_paths.perception_path().display(),
        reservoir_ws = %cli.reservoir_ws_url,
        "consciousness bridge starting"
    );

    if let Err(error) = condition_metrics::ensure_bridge_metrics_file() {
        warn!(error = %error, "failed to initialize condition metrics ledger");
    }

    // Open SQLite database.
    let db = Arc::new(BridgeDb::open(&cli.db_path)?);
    info!("SQLite database opened at {}", cli.db_path);

    // Shared state.
    let state = Arc::new(RwLock::new(BridgeState::new()));

    // Shutdown signal.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Sensory outbound channel — MCP tools and WASM component send here.
    let (sensory_tx, sensory_rx) = mpsc::channel(256);

    // Spawn WebSocket tasks.
    let telemetry_handle = ws::spawn_telemetry_subscriber(
        cli.minime_telemetry.clone(),
        Arc::clone(&state),
        Arc::clone(&db),
        shutdown_rx.clone(),
    );

    let sensory_handle = ws::spawn_sensory_sender(
        cli.minime_sensory.clone(),
        Arc::clone(&state),
        Arc::clone(&db),
        sensory_rx,
        shutdown_rx.clone(),
    );

    // Spawn MCP server on stdio.
    let sensory_tx_mcp = sensory_tx.clone();
    let mcp_handle = tokio::spawn(mcp::run_mcp_server(
        Arc::clone(&state),
        Arc::clone(&db),
        sensory_tx_mcp,
        shutdown_rx.clone(),
    ));

    // Spawn autonomous feedback loop (if enabled).
    let _autonomous_handle = if cli.autonomous {
        let interval = std::time::Duration::from_secs(cli.auto_interval_secs);
        Some(autonomous::spawn_autonomous_loop(
            interval,
            Arc::clone(&state),
            Arc::clone(&db),
            sensory_tx,
            shutdown_rx.clone(),
            Some(resolved_paths.minime_workspace().to_path_buf()),
            Some(resolved_paths.perception_path().to_path_buf()),
        ))
    } else {
        drop(sensory_tx); // Not needed if no autonomous loop.
        None
    };

    // Spawn periodic maintenance: vacuum SQLite every 6 hours.
    let vacuum_db = Arc::clone(&db);
    let mut vacuum_shutdown = shutdown_rx;
    let _vacuum_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
        interval.tick().await; // Skip the immediate first tick.
        loop {
            tokio::select! {
                _ = vacuum_shutdown.changed() => return,
                _ = interval.tick() => {
                    if let Err(e) = vacuum_db.vacuum() {
                        tracing::warn!(error = %e, "periodic vacuum failed");
                    } else {
                        tracing::debug!("periodic vacuum completed");
                    }
                }
            }
        }
    });

    info!("consciousness bridge running — WebSocket + MCP tasks spawned");

    // Wait for shutdown: ctrl-c always, MCP exit only when not autonomous.
    if cli.autonomous {
        // In autonomous mode, don't exit on stdin close — run until ctrl-c.
        tokio::signal::ctrl_c().await?;
        info!("consciousness bridge received ctrl-c");
    } else {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("consciousness bridge received ctrl-c");
            }
            _ = mcp_handle => {
                info!("MCP server exited (stdin closed)");
            }
        }
    }

    info!("consciousness bridge shutting down");

    // Signal all tasks to stop.
    let _ = shutdown_tx.send(true);

    // Wait for WebSocket tasks to finish.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let _ = telemetry_handle.await;
        let _ = sensory_handle.await;
    })
    .await;

    // Purge old messages on graceful shutdown.
    #[expect(clippy::cast_precision_loss)]
    let retention = cli.retention_secs as f64;
    let purged = db.purge_old_messages(retention)?;
    if purged > 0 {
        info!(purged, "purged old messages on shutdown");
    }

    info!("consciousness bridge stopped");
    Ok(())
}
