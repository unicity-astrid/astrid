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

    // Headless mode: -p "prompt" sends a single prompt and exits.
    if let Some(prompt_text) = cli.prompt {
        ensure_global_config();
        return run_headless(prompt_text, output_format, cli.auto_approve).await;
    }

    // Also detect piped stdin with no subcommand as headless.
    if cli.command.is_none() && !std::io::stdin().is_terminal() {
        ensure_global_config();
        let mut stdin_text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin_text)?;
        if !stdin_text.is_empty() {
            return run_headless(stdin_text, output_format, cli.auto_approve).await;
        }
    }

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
            } => {
                commands::capsule::remove::remove_capsule(&name, workspace, force)?;
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
            let socket_path = socket_client::proxy_socket_path();

            // Check if daemon is already running
            if socket_path.exists() {
                if let Ok(_stream) = tokio::net::UnixStream::connect(&socket_path).await {
                    println!(
                        "{}",
                        theme::Theme::warning("Astrid daemon is already running.")
                    );
                    return Ok(());
                }
                // Stale socket — clean up
                let _ = std::fs::remove_file(&socket_path);
                let _ = std::fs::remove_file(socket_client::readiness_path());
            }

            let ready_path = socket_client::readiness_path();
            spawn_persistent_daemon(&ready_path).await?;
        },
        Some(Commands::Status) => {
            let socket_path = socket_client::proxy_socket_path();
            if !socket_path.exists() {
                println!("{}", theme::Theme::info("No Astrid daemon is running."));
                return Ok(());
            }

            // Connect and send GetStatus request
            let session_id = astrid_core::SessionId::from_uuid(uuid::Uuid::new_v4());
            match socket_client::SocketClient::connect(session_id).await {
                Ok(mut client) => {
                    let req = astrid_types::kernel::KernelRequest::GetStatus;
                    if let Ok(val) = serde_json::to_value(req) {
                        let msg = astrid_types::ipc::IpcMessage::new(
                            "astrid.v1.request.status",
                            astrid_types::ipc::IpcPayload::RawJson(val),
                            uuid::Uuid::nil(),
                        );
                        client.send_message(msg).await?;

                        if let Some(response) = client.read_message().await?
                            && let astrid_types::ipc::IpcPayload::RawJson(val) = response.payload
                            && let Ok(astrid_types::kernel::KernelResponse::Status(status)) =
                                serde_json::from_value::<astrid_types::kernel::KernelResponse>(val)
                        {
                            let uptime_display = format_uptime(status.uptime_secs);
                            println!(
                                "{}",
                                theme::Theme::success(&format!(
                                    "Astrid daemon (PID {}, uptime {})",
                                    status.pid, uptime_display
                                ))
                            );
                            println!("  Version:    {}", status.version);
                            println!("  Clients:    {}", status.connected_clients);
                            println!("  Capsules:   {} loaded", status.loaded_capsules.len());
                            for capsule in &status.loaded_capsules {
                                println!("    - {capsule}");
                            }
                        } else {
                            println!("{}", theme::Theme::error("Unexpected response from daemon"));
                        }
                    }
                },
                Err(_) => {
                    println!(
                        "{}",
                        theme::Theme::error(
                            "Daemon socket exists but connection failed. \
                             It may be starting up or in a bad state."
                        )
                    );
                },
            }
        },
        Some(Commands::Stop) => {
            let socket_path = socket_client::proxy_socket_path();
            if !socket_path.exists() {
                println!("{}", theme::Theme::info("No Astrid daemon is running."));
                return Ok(());
            }

            let session_id = astrid_core::SessionId::from_uuid(uuid::Uuid::new_v4());
            if let Ok(mut client) = socket_client::SocketClient::connect(session_id).await {
                let req = astrid_types::kernel::KernelRequest::Shutdown {
                    reason: Some("astrid stop".to_string()),
                };
                if let Ok(val) = serde_json::to_value(req) {
                    let msg = astrid_types::ipc::IpcMessage::new(
                        "astrid.v1.request.shutdown",
                        astrid_types::ipc::IpcPayload::RawJson(val),
                        uuid::Uuid::nil(),
                    );
                    client.send_message(msg).await?;
                    println!("{}", theme::Theme::success("Astrid daemon stopped."));
                }
            } else {
                // Socket exists but can't connect — stale. Clean up.
                let _ = std::fs::remove_file(&socket_path);
                let _ = std::fs::remove_file(socket_client::readiness_path());
                println!("{}", theme::Theme::info("Cleaned up stale daemon socket."));
            }
        },
    }

    Ok(())
}

/// Build a hint string pointing the user to the daemon log directory.
fn daemon_log_hint() -> String {
    astrid_core::dirs::AstridHome::resolve()
        .map(|h| format!(" Check logs: {}", h.log_dir().display()))
        .unwrap_or_default()
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

/// Spawn the daemon process and wait for it to signal readiness.
///
/// Returns the child process handle on success. The caller must `drop()` it
/// after a successful handshake (to disown), or `kill()` + `wait()` on failure.
///
/// # Errors
/// Returns an error if the daemon binary is not found, fails to spawn, or
/// doesn't become ready within 10 seconds.
async fn spawn_daemon(ready_path: &std::path::Path) -> Result<std::process::Child> {
    println!("{}", theme::Theme::info("Booting Astrid daemon..."));
    let ws = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let daemon_bin = find_companion_binary("astrid-daemon")?;

    let mut cmd = std::process::Command::new(daemon_bin);
    cmd.arg("--ephemeral");

    if let Some(ws_path) = ws.to_str() {
        cmd.arg("--workspace").arg(ws_path);
    }

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // Remove stale readiness file before spawning so we don't
    // mistake a leftover from a crashed daemon for the new one.
    let _ = std::fs::remove_file(ready_path);

    let mut child = cmd
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
        // If the daemon has already exited, stop polling immediately
        // instead of waiting the full 10 seconds.
        if let Ok(Some(status)) = child.try_wait() {
            anyhow::bail!("Daemon exited prematurely ({status}).{}", daemon_log_hint());
        }
    }
    if !ready {
        // Kill the child to prevent an orphan daemon that lingers
        // until its idle timeout expires.
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!(
            "Daemon failed to become ready within 10 seconds.{}",
            daemon_log_hint()
        );
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

/// Spawn a persistent (non-ephemeral) daemon and wait for readiness.
async fn spawn_persistent_daemon(ready_path: &std::path::Path) -> Result<()> {
    println!(
        "{}",
        theme::Theme::info("Starting Astrid daemon (persistent mode)...")
    );
    let ws = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let daemon_bin = find_companion_binary("astrid-daemon")?;

    let mut cmd = std::process::Command::new(daemon_bin);
    // No --ephemeral flag = persistent mode

    if let Some(ws_path) = ws.to_str() {
        cmd.arg("--workspace").arg(ws_path);
    }

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let _ = std::fs::remove_file(ready_path);

    let mut child = cmd.spawn().context("Failed to spawn Astrid daemon")?;

    let mut ready = false;
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if ready_path.exists() {
            ready = true;
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            anyhow::bail!("Daemon exited prematurely ({status}).{}", daemon_log_hint());
        }
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!(
            "Daemon failed to become ready within 10 seconds.{}",
            daemon_log_hint()
        );
    }

    // Disown the child — it runs independently.
    drop(child);

    println!(
        "{}",
        theme::Theme::success("Astrid daemon started (persistent mode).")
    );
    Ok(())
}

/// Headless mode: send a single prompt, stream the response to stdout, exit.
///
/// Connects to the daemon (spawning if needed), sends the prompt as a
/// `UserInput` IPC message, and reads response events until the final
/// `AgentResponse` with `is_final = true`.
///
/// Output format:
/// - `Pretty`: prints the raw response text to stdout.
/// - `Json`: prints a JSON object with `response` and tool call details.
async fn run_headless(
    prompt: String,
    format: formatter::OutputFormat,
    auto_approve: bool,
) -> Result<()> {
    use astrid_core::SessionId;

    let socket_path = socket_client::proxy_socket_path();
    let ready_path = socket_client::readiness_path();

    // Boot daemon if needed
    let needs_boot = if socket_path.exists() {
        if let Ok(_stream) = tokio::net::UnixStream::connect(&socket_path).await {
            eprintln!("[headless] Connected to existing daemon");
            false
        } else {
            eprintln!("[headless] Stale socket, respawning daemon...");
            let _ = std::fs::remove_file(&socket_path);
            let _ = std::fs::remove_file(&ready_path);
            true
        }
    } else {
        true
    };
    if needs_boot {
        spawn_daemon(&ready_path).await?;
    }

    let session_id = SessionId::from_uuid(uuid::Uuid::new_v4());
    let mut client = socket_client::SocketClient::connect(session_id.clone())
        .await
        .context("Failed to connect to daemon")?;

    // Also read stdin if there's piped content and -p was used
    let full_prompt = if std::io::stdin().is_terminal() {
        prompt
    } else {
        let mut stdin_text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin_text)?;
        if stdin_text.is_empty() {
            prompt
        } else {
            format!("{stdin_text}\n\n{prompt}")
        }
    };

    // Send the prompt and collect the streaming response
    client.send_input(full_prompt).await?;
    let (response_text, tool_calls) =
        collect_headless_response(&mut client, &session_id, format, auto_approve).await?;

    // Final output
    match format {
        formatter::OutputFormat::Pretty => {
            if !response_text.ends_with('\n') {
                println!();
            }
        },
        formatter::OutputFormat::Json => {
            let output = serde_json::json!({
                "response": response_text,
                "tool_calls": tool_calls,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        },
    }

    // Send disconnect
    let disconnect = astrid_types::ipc::IpcMessage::new(
        "client.v1.disconnect",
        astrid_types::ipc::IpcPayload::Disconnect {
            reason: Some("headless".to_string()),
        },
        session_id.0,
    );
    let _ = client.send_message(disconnect).await;

    Ok(())
}

/// Collect the streaming response from the daemon in headless mode.
///
/// Returns `(response_text, tool_calls)`. Auto-denies any approval requests.
/// Times out after 120 seconds of no data.
async fn collect_headless_response(
    client: &mut socket_client::SocketClient,
    session_id: &astrid_core::SessionId,
    format: formatter::OutputFormat,
    auto_approve: bool,
) -> Result<(String, Vec<serde_json::Value>)> {
    let mut response_text = String::new();
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();
    let timeout_duration = std::time::Duration::from_secs(120);

    loop {
        let message = match tokio::time::timeout(timeout_duration, client.read_message()).await {
            Ok(Ok(Some(msg))) => msg,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(e.context("Failed to read from daemon")),
            Err(_) => {
                eprintln!("[headless] Timed out waiting for response (120s)");
                std::process::exit(53);
            },
        };

        match &message.payload {
            astrid_types::ipc::IpcPayload::AgentResponse { text, is_final, .. } => {
                if format == formatter::OutputFormat::Pretty {
                    print!("{text}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                response_text.push_str(text);
                if *is_final {
                    break;
                }
            },
            astrid_types::ipc::IpcPayload::LlmStreamEvent {
                event: astrid_types::llm::StreamEvent::ToolCallStart { id, name },
                ..
            } => {
                tool_calls.push(serde_json::json!({
                    "type": "tool_call",
                    "id": id,
                    "name": name,
                }));
            },
            astrid_types::ipc::IpcPayload::ToolExecuteResult { call_id, result } => {
                tool_calls.push(serde_json::json!({
                    "type": "tool_result",
                    "call_id": call_id,
                    "content": result.content,
                    "is_error": result.is_error,
                }));
            },
            astrid_types::ipc::IpcPayload::ApprovalRequired {
                request_id, action, ..
            } => {
                let decision = if auto_approve { "approve" } else { "deny" };
                eprintln!(
                    "[headless] Auto-{} approval for: {action}",
                    if auto_approve { "approved" } else { "denied" }
                );
                let response = astrid_types::ipc::IpcPayload::ApprovalResponse {
                    request_id: request_id.clone(),
                    decision: decision.to_string(),
                    reason: Some(
                        if auto_approve {
                            "headless --yes mode"
                        } else {
                            "headless mode"
                        }
                        .to_string(),
                    ),
                };
                let topic = format!("astrid.v1.approval.response.{request_id}");
                let msg = astrid_types::ipc::IpcMessage::new(topic, response, session_id.0);
                client.send_message(msg).await?;
            },
            _ => {},
        }
    }

    Ok((response_text, tool_calls))
}

/// Format seconds into a human-readable uptime string.
fn format_uptime(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m{seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}
