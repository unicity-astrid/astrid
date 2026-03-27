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
#![expect(
    dead_code,
    reason = "incremental development — some plumbing used by later features"
)]

use std::io::IsTerminal;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod commands;
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
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Output format: pretty (default), json, or stream-json
    #[arg(long, global = true, default_value = "pretty")]
    format: String,

    /// Non-interactive prompt. Sends the prompt, prints the response, and exits.
    /// Forces headless mode (no TUI). Stdin is appended to the prompt if piped.
    #[arg(short, long)]
    prompt: Option<String>,

    /// Auto-approve all tool approval requests in headless mode (autonomous/yolo mode).
    /// Without this flag, headless mode auto-denies approvals.
    #[arg(short = 'y', long = "yes", alias = "yolo", alias = "autonomous")]
    auto_approve: bool,

    /// Resume or create a named session for multi-turn headless conversations.
    /// Use the same ID across multiple -p calls to maintain context.
    /// If omitted, a fresh session is created each time.
    #[arg(long = "session")]
    session_name: Option<String>,

    /// Print the session ID to stderr after the response, for use in scripts.
    #[arg(long = "print-session")]
    print_session: bool,

    /// Render the TUI to stdout as text snapshots instead of an interactive terminal.
    /// Each significant event (input, response, tool call, approval) produces a frame.
    /// Requires --prompt. Useful for automated testing and CI.
    #[arg(long = "snapshot-tui")]
    snapshot_tui: bool,

    /// Terminal width for --snapshot-tui rendering (default: 120).
    #[arg(long = "tui-width", default_value = "120")]
    tui_width: u16,

    /// Terminal height for --snapshot-tui rendering (default: 40).
    #[arg(long = "tui-height", default_value = "40")]
    tui_height: u16,

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

    /// Initialize a workspace and install a distro
    Init {
        /// Distro to install (name, @org/repo, or path to Distro.toml)
        #[arg(long, default_value = "astralis")]
        distro: String,
    },

    /// Start the Astrid daemon in persistent mode (detached, no TUI)
    Start,

    /// Show daemon status (PID, uptime, connected clients, loaded capsules)
    Status,

    /// Stop a running Astrid daemon
    Stop,

    /// Update Astrid to the latest release
    SelfUpdate,
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
    /// Remove an installed capsule
    Remove {
        /// Capsule name to remove
        name: String,
        /// Remove from workspace instead of user-level
        #[arg(long)]
        workspace: bool,
        /// Force removal even if other capsules depend on it
        #[arg(long)]
        force: bool,
        /// Also delete saved configuration (API keys, env vars)
        #[arg(long)]
        purge: bool,
    },
    /// Show the capsule imports/exports dependency tree
    Tree,
    /// Alias for `tree` (deprecated)
    #[command(hide = true)]
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

// ─── Bootstrap helpers ───────────────────────────────────────────

fn ensure_global_config() {
    use astrid_core::dirs::AstridHome;
    if let Ok(home) = AstridHome::resolve() {
        let _ = home.ensure();
    }
    // Auto-init on first run.
    let _ = ensure_initialized();
}

/// Run `astrid init` automatically if no distro has been installed yet.
///
/// Checks for `distro.lock` as the canonical signal. If absent, runs init
/// with the default distro so first-time users don't need a separate step.
fn ensure_initialized() -> Result<()> {
    if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
        let principal = astrid_core::PrincipalId::default();
        let lock_path = home
            .principal_home(&principal)
            .config_dir()
            .join("distro.lock");
        if !lock_path.exists() {
            eprintln!(
                "{}",
                theme::Theme::info("First run detected — running astrid init...")
            );
            commands::init::run_init("astralis")?;
            commands::self_update::ensure_path_setup()?;
        }
    }
    Ok(())
}

fn init_logging(cli: &Cli) {
    let workspace_root = std::env::current_dir().ok();
    let unified_cfg = astrid_config::Config::load(workspace_root.as_deref())
        .ok()
        .map(|r| r.config);

    let needs_file_log = matches!(cli.command, Some(Commands::Chat { .. }) | None);

    let log_config = if let Some(cfg) = &unified_cfg {
        let mut lc = astrid_telemetry::log_config_from(cfg);
        if cli.verbose {
            "debug".clone_into(&mut lc.level);
        }
        if needs_file_log && let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            lc.target = astrid_telemetry::LogTarget::File(home.log_dir());
        }
        lc
    } else {
        let level = if cli.verbose { "debug" } else { "info" };
        let mut lc = astrid_telemetry::LogConfig::new(level)
            .with_format(astrid_telemetry::LogFormat::Compact);
        if needs_file_log && let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
            lc.target = astrid_telemetry::LogTarget::File(home.log_dir());
        }
        lc
    };

    if let Err(e) = astrid_telemetry::setup_logging(&log_config) {
        eprintln!("Failed to initialize logging: {e}");
    }
}

/// Locate a companion binary (e.g. `astrid-daemon`, `astrid-build`).
///
/// Search order:
/// 1. Same directory as the current executable (co-installed)
/// 2. `PATH` lookup
pub(crate) fn find_companion_binary(name: &str) -> Result<std::path::PathBuf> {
    // 1. Check next to the CLI binary
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    // 2. PATH lookup
    if let Ok(path) = which::which(name) {
        return Ok(path);
    }

    anyhow::bail!(
        "{name} not found. Ensure it is installed alongside the astrid CLI \
         or available in PATH."
    )
}

// ─── Main dispatch ───────────────────────────────────────────────

#[tokio::main]
#[expect(clippy::too_many_lines, reason = "top-level command dispatch")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    init_logging(&cli);

    // Check for updates (cached, non-blocking) on interactive commands.
    if cli.prompt.is_none() && !matches!(cli.command, Some(Commands::SelfUpdate)) {
        commands::self_update::print_update_banner();
    }

    // Parse output format.
    let output_format = match cli.format.as_str() {
        "json" => formatter::OutputFormat::Json,
        _ => formatter::OutputFormat::Pretty,
    };

    // Headless mode: -p "prompt" sends a single prompt and exits.
    if let Some(prompt_text) = cli.prompt {
        ensure_global_config();
        if cli.snapshot_tui {
            return commands::headless::run_snapshot_tui(
                prompt_text,
                cli.auto_approve,
                cli.session_name,
                cli.tui_width,
                cli.tui_height,
            )
            .await;
        }
        return commands::headless::run_headless(
            prompt_text,
            output_format,
            cli.auto_approve,
            cli.session_name,
            cli.print_session,
        )
        .await;
    }

    // Also detect piped stdin with no subcommand as headless.
    if cli.command.is_none() && !std::io::stdin().is_terminal() {
        ensure_global_config();
        let mut stdin_text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin_text)?;
        if !stdin_text.is_empty() {
            return commands::headless::run_headless(
                stdin_text,
                output_format,
                cli.auto_approve,
                cli.session_name,
                cli.print_session,
            )
            .await;
        }
    }

    // Subcommand dispatch.
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
            if output_format == formatter::OutputFormat::Json {
                print_banner();
            }
            ensure_global_config();
            let workspace = std::env::current_dir().ok();
            run_or_connect(None, workspace, output_format).await?;
        },
        Some(Commands::Build {
            path,
            output,
            project_type,
            from_mcp_json,
        }) => {
            let build_bin = find_companion_binary("astrid-build")?;
            let mut cmd = std::process::Command::new(build_bin);
            if let Some(p) = &path {
                cmd.arg(p);
            }
            if let Some(o) = &output {
                cmd.arg("--output").arg(o);
            }
            if let Some(t) = &project_type {
                cmd.arg("--type").arg(t);
            }
            if let Some(m) = &from_mcp_json {
                cmd.arg("--from-mcp-json").arg(m);
            }
            let status = cmd.status().context("Failed to run astrid-build")?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        },
        Some(Commands::Init { distro }) => {
            commands::init::run_init(&distro)?;
            commands::self_update::ensure_path_setup()?;
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
            CapsuleCommands::Remove {
                name,
                workspace,
                force,
                purge,
            } => {
                commands::capsule::remove::remove_capsule(&name, workspace, force, purge)?;
            },
            CapsuleCommands::Tree | CapsuleCommands::Deps => {
                commands::capsule::deps::show_tree()?;
            },
        },
        Some(Commands::Session { command }) => {
            commands::sessions::handle_session_commands(command)?;
        },
        Some(Commands::Start) => {
            ensure_global_config();
            commands::daemon::handle_start().await?;
        },
        Some(Commands::Status) => {
            commands::daemon::handle_status().await?;
        },
        Some(Commands::Stop) => {
            commands::daemon::handle_stop().await?;
        },
        Some(Commands::SelfUpdate) => {
            commands::self_update::run_self_update()?;
        },
    }

    Ok(())
}

// ─── Interactive session ─────────────────────────────────────────

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
        match commands::daemon::spawn_daemon(&ready_path).await {
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
                        h.log_dir().display()
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
