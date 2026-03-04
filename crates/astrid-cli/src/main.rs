//! Astrid CLI - Secure Agent Runtime
//!
//! A production-grade secure agent runtime with proper security from day one.
//! The CLI is a thin client: it connects to the kernel (auto-starting if needed),
//! creates/resumes sessions, and renders streaming events.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![allow(dead_code)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

pub mod config_bridge;
/// The socket client for interacting with the Kernel.
pub mod socket_client;
mod formatter;
mod repl;
mod theme;
mod commands;
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

    /// Run the Astrid OS Kernel in the background for a specific session
    Daemon {
        /// The session ID to bind the Kernel to
        #[arg(short, long)]
        session: String,
        
        /// Optional workspace root directory
        #[arg(short, long)]
        workspace: Option<std::path::PathBuf>,
    },

    /// Initialize a workspace
    Init,
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
}

fn ensure_global_config() {
    use astrid_core::dirs::AstridHome;
    if let Ok(home) = AstridHome::resolve() {
        let _ = home.ensure();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load unified config for logging setup.
    let workspace_root = std::env::current_dir().ok();
    let unified_cfg = astrid_config::Config::load(workspace_root.as_deref())
        .ok()
        .map(|r| r.config);

    // Set up logging from config, with --verbose override.
        let log_config = if let Some(cfg) = &unified_cfg {
        let mut lc = config_bridge::to_log_config(cfg);
        if cli.verbose {
            "debug".clone_into(&mut lc.level);
        }
        // Force file logging if running interactive Chat mode to prevent TUI corruption
        if matches!(cli.command, Some(Commands::Chat { .. }) | None)
            && let Ok(home) = astrid_core::dirs::AstridHome::resolve()
        {
            lc.target = astrid_telemetry::LogTarget::File(home.logs_dir());
        }
        lc
    } else {
        // Fallback if config loading fails.
        let level = if cli.verbose { "debug" } else { "info" };
        let mut lc = astrid_telemetry::LogConfig::new(level)
            .with_format(astrid_telemetry::LogFormat::Compact);
        if matches!(cli.command, Some(Commands::Chat { .. }) | None)
            && let Ok(home) = astrid_core::dirs::AstridHome::resolve()
        {
            lc.target = astrid_telemetry::LogTarget::File(home.logs_dir());
        }
        lc
    };
    if let Err(e) = astrid_telemetry::setup_logging(&log_config) {
        eprintln!("Failed to initialize logging: {e}");
    }

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
                uuid::Uuid::parse_str(&session).map_err(|e| anyhow::anyhow!("Invalid UUID format: {e}"))?
            );
            let ws = workspace.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
            
            let kernel = astrid_kernel::Kernel::new(session_id.clone(), ws).await
                .map_err(|e| anyhow::anyhow!("Failed to boot local Kernel: {e}"))?;
                
            // Load all plugins (auto-discovery)
            kernel.load_all_capsules().await;
            
            println!("{}", theme::Theme::success(&format!("Kernel successfully booted for session {}", session_id.0)));
            
            // Sleep forever to keep the Kernel alive in the background
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
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

        Some(Commands::Capsule { command }) => {
            match command {
                CapsuleCommands::Install { source, workspace } => {
                    commands::capsule::install::install_capsule(&source, workspace)?;
                },
            }
        },
    }

    Ok(())
}

/// The core Host wrapper logic.
/// Resolves the session, checks for an existing socket, and boots the kernel locally if necessary.
///
/// # Errors
/// Returns an error if the kernel fails to boot or the socket fails to connect.
pub(crate) async fn run_or_connect(
    session: Option<String>,
    workspace: Option<std::path::PathBuf>,
    format: formatter::OutputFormat,
) -> Result<()> {
    use astrid_core::SessionId;
    use uuid::Uuid;

    // 1. Resolve Session ID
    let session_id = if let Some(sid) = session {
        SessionId::from_uuid(Uuid::parse_str(&sid).map_err(|e| anyhow::anyhow!("Invalid UUID format: {e}"))?)
    } else {
        SessionId::from_uuid(Uuid::new_v4())
    };

    let socket_path = socket_client::proxy_socket_path(&session_id);

    // 2. Check if a Kernel is already running for this session
    let mut needs_boot = !socket_path.exists();

    if socket_path.exists() {
        // Test if the socket is actually alive by attempting a connection
        match tokio::net::UnixStream::connect(&socket_path).await {
            Ok(_) => {
                println!("{}", theme::Theme::info(&format!("Connecting to existing Kernel at Session {}", session_id.0)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                println!("{}", theme::Theme::warning("Found dead session socket. Cleaning up and rebooting kernel..."));
                let _ = std::fs::remove_file(&socket_path);
                needs_boot = true;
            }
            Err(e) => {
                anyhow::bail!("Failed to check session socket: {e}");
            }
        }
    }

    if needs_boot {
        println!("{}", theme::Theme::info(&format!("Booting Local Kernel for Session {}", session_id.0)));
        let ws = workspace.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
        
        let exe = std::env::current_exe().context("Failed to get current executable path")?;
        
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("daemon")
           .arg("--session")
           .arg(session_id.0.to_string());
           
        if let Some(ws_path) = ws.to_str() {
            cmd.arg("--workspace").arg(ws_path);
        }
        
        // Detach the process from the current terminal's standard I/O
        cmd.stdin(std::process::Stdio::null())
           .stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::null());
           
        // Spawn the background process
        let child = cmd.spawn().context("Failed to spawn background Kernel daemon")?;
        
        // Disown the child so it survives when the CLI exits
        // Note: Unix requires a double fork for true disowning, but dropping the child 
        // handle prevents the CLI from wait()ing on it.
        std::mem::drop(child);
        
        // Wait a tiny moment for the socket task to bind in the background process
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // 3. Connect the dumb pipe
    let mut client = socket_client::SocketClient::connect(session_id.clone()).await?;
    
    // 4. Run the TUI or simple REPL loop
    let workspace_root = std::env::current_dir().ok();
    let model_name = astrid_config::Config::load(workspace_root.as_deref())
        .ok()
        .map_or_else(|| "unknown".to_string(), |r| r.config.model.model);
        
    crate::commands::chat::run_chat(&mut client, &session_id, &model_name, format).await
}