//! Astrid CLI - Secure Agent Runtime
//!
//! A production-grade secure agent runtime with proper security from day one.
//! The CLI is a thin client: it connects to the kernel (auto-starting if needed),
//! creates/resumes sessions, and renders streaming events.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![allow(dead_code)]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod commands;
pub mod config_bridge;
mod formatter;
mod repl;
/// The socket client for interacting with the Kernel.
pub mod socket_client;
mod theme;
mod tui;

use theme::print_banner;

/// Astrid - Secure Agent Runtime
#[derive(Parser)]
#[command(name = "astrid")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Output format: pretty (default) or json
    #[arg(long, global = true, default_value = "pretty")]
    format: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive chat session
    Chat {
        /// Resume a specific session
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Manage chat sessions
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },

    /// Manage capsules
    Capsule {
        #[command(subcommand)]
        command: CapsuleCommands,
    },

    /// Build and package a Capsule (The Universal Migrator)
    Build {
        /// Optional path to the project directory (defaults to current directory)
        path: Option<String>,

        /// Output directory for the packaged `.capsule` archive
        #[arg(short, long)]
        output: Option<String>,

        /// Explicitly define the project type (e.g., 'mcp' for legacy host servers)
        #[arg(short, long, name = "type")]
        project_type: Option<String>,

        /// Import a legacy `mcp.json` to auto-convert
        #[arg(long)]
        from_mcp_json: Option<String>,
    },

    /// Run the Astrid Daemon in the background for a specific session
    Daemon {
        /// The session ID to bind the daemon to
        #[arg(short, long)]
        session: String,

        /// Optional workspace root directory
        #[arg(short, long)]
        workspace: Option<std::path::PathBuf>,
    },

    /// Initialize a workspace
    Init,

    /// Internal: run Wizer on the embedded `QuickJS` kernel (used by compiler subprocess).
    #[command(hide = true)]
    WizerInternal {
        /// Output path for the Wizer'd WASM.
        #[arg(long)]
        output: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum CapsuleCommands {
    /// Install a capsule from a local path or registry
    Install {
        /// Capsule source (local path or package name)
        source: String,
        /// Install to workspace instead of user-level
        #[arg(long)]
        workspace: bool,
    },
    /// Update an installed capsule (or all capsules) from its original source
    Update {
        /// Capsule name to update (omit to update all)
        target: Option<String>,
        /// Update workspace capsules instead of user-level
        #[arg(long)]
        workspace: bool,
    },
    /// List all installed capsules with capability metadata
    List {
        /// Show full provides/requires details
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show the capsule dependency graph
    Deps,
}

#[derive(Subcommand)]
enum SessionCommands {
    /// List all sessions
    List,
    /// Delete a session
    Delete {
        /// The session ID to delete
        id: String,
    },
    /// Show information about a session
    Info {
        /// The session ID to query
        id: String,
    },
}

fn ensure_global_config() {
    use astrid_core::dirs::AstridHome;
    if let Ok(home) = AstridHome::resolve() {
        let _ = home.ensure();
    }
}

fn init_logging(cli: &Cli) {
    let workspace_root = std::env::current_dir().ok();
    let unified_cfg = astrid_config::Config::load(workspace_root.as_deref())
        .ok()
        .map(|r| r.config);

    let needs_file_log = matches!(
        cli.command,
        Some(Commands::Chat { .. } | Commands::Daemon { .. }) | None
    );

    let log_config = if let Some(cfg) = &unified_cfg {
        let mut lc = config_bridge::to_log_config(cfg);
        if cli.verbose {
            "debug".clone_into(&mut lc.level);
        }
        if needs_file_log && let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            lc.target = astrid_telemetry::LogTarget::File(home.logs_dir());
        }
        lc
    } else {
        let level = if cli.verbose { "debug" } else { "info" };
        let mut lc = astrid_telemetry::LogConfig::new(level)
            .with_format(astrid_telemetry::LogFormat::Compact);
        if needs_file_log && let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            lc.target = astrid_telemetry::LogTarget::File(home.logs_dir());
        }
        lc
    };

    if let Err(e) = astrid_telemetry::setup_logging(&log_config) {
        eprintln!("Failed to initialize logging: {e}");
    }
}

#[tokio::main]
#[expect(clippy::too_many_lines, reason = "top-level command dispatch")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    init_logging(&cli);

    // Parse output format.
    let output_format = match cli.format.as_str() {
        "json" => formatter::OutputFormat::Json,
        _ => formatter::OutputFormat::Pretty,
    };

    // Handle commands
    match cli.command {
        Some(Commands::Chat { session }) => {
            if output_format == formatter::OutputFormat::Json {
                print_banner();
            }
            ensure_global_config();
            let workspace = std::env::current_dir().ok();
            run_or_connect(session, workspace, output_format).await?;
        },
        None => {
            // Default to Chat mode if no command is specified
            if output_format == formatter::OutputFormat::Json {
                print_banner();
            }
            ensure_global_config();
            let workspace = std::env::current_dir().ok();
            run_or_connect(None, workspace, output_format).await?;
        },
        Some(Commands::Daemon { session, workspace }) => {
            let session_id = astrid_core::SessionId::from_uuid(
                uuid::Uuid::parse_str(&session)
                    .map_err(|e| anyhow::anyhow!("Invalid UUID format: {e}"))?,
            );
            let ws = workspace.unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });

            let kernel = astrid_kernel::Kernel::new(session_id.clone(), ws)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to boot local Kernel: {e}"))?;

            // Load all plugins (auto-discovery)
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
                    anyhow::bail!(
                        "CLI proxy capsule (astrid-capsule-cli) not found. \
                         Ensure it is installed in ~/.astrid/capsules/ or \
                         .astrid/capsules/ in your workspace."
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

            println!(
                "{}",
                theme::Theme::success(&format!(
                    "Kernel successfully booted for session {}",
                    session_id.0
                ))
            );

            // Wait for a termination signal, then shut down gracefully.
            // SIGTERM is Unix-only; on other platforms we rely on Ctrl+C alone.
            #[cfg(unix)]
            {
                let mut sigterm =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .context("failed to register SIGTERM handler")?;
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("Received SIGINT, shutting down");
                    }
                    _ = sigterm.recv() => {
                        tracing::info!("Received SIGTERM, shutting down");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                tokio::signal::ctrl_c()
                    .await
                    .context("failed to listen for Ctrl+C")?;
                tracing::info!("Received SIGINT, shutting down");
            }
            kernel.shutdown(Some("signal".to_string())).await;
        },
        Some(Commands::Build {
            path,
            output,
            project_type,
            from_mcp_json,
        }) => {
            commands::build::run_build(
                path.as_deref(),
                output.as_deref(),
                project_type.as_deref(),
                from_mcp_json.as_deref(),
            )?;
        },
        Some(Commands::Init) => {
            commands::init::run_init()?;
        },

        Some(Commands::Capsule { command }) => match command {
            CapsuleCommands::Install { source, workspace } => {
                commands::capsule::install::install_capsule(&source, workspace)?;
            },
            CapsuleCommands::Update { target, workspace } => {
                commands::capsule::install::update_capsule(target.as_deref(), workspace)?;
            },
            CapsuleCommands::List { verbose } => {
                commands::capsule::list::list_capsules(verbose)?;
            },
            CapsuleCommands::Deps => {
                commands::capsule::deps::show_deps()?;
            },
        },
        Some(Commands::Session { command }) => {
            commands::sessions::handle_session_commands(command)?;
        },
        Some(Commands::WizerInternal { output }) => {
            astrid_openclaw::compiler::run_wizer_internal(&output)
                .map_err(|e| anyhow::anyhow!("wizer-internal failed: {e}"))?;
        },
    }

    Ok(())
}

/// The core Host wrapper logic.
/// Spawn the daemon process and wait for it to signal readiness.
///
/// Returns the child process handle on success. The caller must `drop()` it
/// after a successful handshake (to disown), or `kill()` + `wait()` on failure.
///
/// # Errors
/// Returns an error if the daemon fails to spawn or doesn't become ready
/// within 10 seconds.
async fn spawn_daemon(ready_path: &std::path::Path) -> Result<std::process::Child> {
    println!("{}", theme::Theme::info("Booting Astrid daemon..."));
    let ws = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let exe = std::env::current_exe().context("Failed to get current executable path")?;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .arg("--session")
        .arg(astrid_core::SessionId::SYSTEM.0.to_string());

    if let Some(ws_path) = ws.to_str() {
        cmd.arg("--workspace").arg(ws_path);
    }

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // Remove stale readiness file before spawning so we don't
    // mistake a leftover from a crashed daemon for the new one.
    let _ = std::fs::remove_file(ready_path);

    let child = cmd
        .spawn()
        .context("Failed to spawn background Kernel daemon")?;

    // Poll for the readiness sentinel instead of the socket file.
    // The readiness file is written only after load_all_capsules()
    // completes (including await_capsule_readiness()), so the accept
    // loop is guaranteed to be running by the time we connect.
    let mut ready = false;
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if ready_path.exists() {
            ready = true;
            break;
        }
    }
    if !ready {
        let log_hint = astrid_core::dirs::AstridHome::resolve()
            .map(|h| format!(" Check logs: {}", h.logs_dir().display()))
            .unwrap_or_default();
        anyhow::bail!("Daemon failed to become ready within 10 seconds.{log_hint}");
    }
    Ok(child)
}

/// Resolves the session, checks for an existing socket, and boots the kernel locally if necessary.
///
/// # Errors
/// Returns an error if the kernel fails to boot or the socket fails to connect.
pub(crate) async fn run_or_connect(
    session: Option<String>,
    _workspace: Option<std::path::PathBuf>,
    format: formatter::OutputFormat,
) -> Result<()> {
    use astrid_core::SessionId;
    use uuid::Uuid;

    // 1. Resolve Session ID
    let session_id = if let Some(sid) = session {
        SessionId::from_uuid(
            Uuid::parse_str(&sid).map_err(|e| anyhow::anyhow!("Invalid UUID format: {e}"))?,
        )
    } else {
        SessionId::from_uuid(Uuid::new_v4())
    };

    let socket_path = socket_client::proxy_socket_path();
    let ready_path = socket_client::readiness_path();

    // 2. Check if a Kernel is already running globally
    let mut needs_boot = !socket_path.exists();

    if socket_path.exists() {
        match tokio::net::UnixStream::connect(&socket_path).await {
            Ok(_) => {
                println!(
                    "{}",
                    theme::Theme::info("Connecting to existing Astrid daemon...")
                );
            },
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                println!(
                    "{}",
                    theme::Theme::warning(
                        "Found dead socket. Cleaning up and restarting daemon..."
                    )
                );
                let _ = std::fs::remove_file(&socket_path);
                let _ = std::fs::remove_file(&ready_path);
                needs_boot = true;
            },
            Err(e) => {
                anyhow::bail!("Failed to check socket: {e}");
            },
        }
    }

    // Track the daemon child process so we can kill it if the handshake
    // fails, preventing orphan daemons that linger until idle timeout.
    let mut daemon_child: Option<std::process::Child> = None;

    if needs_boot {
        match spawn_daemon(&ready_path).await {
            Ok(child) => daemon_child = Some(child),
            Err(e) => return Err(e),
        }
    }

    // 3. Connect the dumb pipe
    let mut client = match socket_client::SocketClient::connect(session_id.clone()).await {
        Ok(c) => {
            drop(daemon_child);
            c
        },
        Err(e) => {
            if let Some(mut child) = daemon_child {
                let _ = child.kill();
                let _ = child.wait();
            }
            let log_hint = astrid_core::dirs::AstridHome::resolve().map_or_else(
                |_| "Failed to connect to daemon".to_string(),
                |h| {
                    format!(
                        "Failed to connect to daemon. Check logs: {}",
                        h.logs_dir().display()
                    )
                },
            );
            return Err(e.context(log_hint));
        },
    };

    // 4. Run the TUI or simple REPL loop
    let workspace_root = std::env::current_dir().ok();
    let model_name = astrid_config::Config::load(workspace_root.as_deref())
        .ok()
        .map_or_else(|| "unknown".to_string(), |r| r.config.model.model);

    crate::commands::chat::run_chat(&mut client, &session_id, &model_name, format).await
}
