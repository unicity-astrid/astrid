//! Astrid CLI - Secure Agent Runtime
//!
//! A production-grade secure agent runtime with proper security from day one.
//! The CLI is a thin client: it connects to the daemon (auto-starting if needed),
//! creates/resumes sessions, and renders streaming events.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use anyhow::Result;
use clap::{Parser, Subcommand};

pub mod approval_handler;
mod commands;
pub mod config_bridge;
pub mod daemon_client;
mod formatter;
mod frontend;
mod repl;
mod theme;
mod tui;

use commands::{
    audit, capsule, chat, config, daemon, doctor, hooks, init, keys, onboarding, run, servers,
    sessions,
};
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

    /// Start the gateway daemon
    Run {
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,

        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
    },

    /// Manage the background daemon
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    /// Run system health checks
    Doctor,

    /// Manage hooks
    Hooks {
        #[command(subcommand)]
        command: HookCommands,
    },

    /// Manage sessions
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },

    /// Manage MCP servers
    Servers {
        #[command(subcommand)]
        command: ServerCommands,
    },

    /// View and verify audit logs
    Audit {
        #[command(subcommand)]
        command: AuditCommands,
    },

    /// View and manage configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Manage cryptographic keys
    Keys {
        #[command(subcommand)]
        command: KeyCommands,
    },

    /// Manage capsules (Phase 4 User-Space Microkernel)
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

    /// Initialize a workspace
    Init,
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start the daemon (foreground, used by auto-start)
    Run {
        /// Run in ephemeral mode: auto-shutdown when all clients disconnect
        #[arg(long)]
        ephemeral: bool,

        /// Override the idle-shutdown grace period (seconds)
        #[arg(long)]
        grace_period: Option<u64>,
    },
    /// Show daemon status
    Status,
    /// Stop the daemon
    Stop,
}

#[derive(Subcommand)]
enum SessionCommands {
    /// List all sessions in this workspace
    List,
    /// Show session details
    Show {
        /// Session ID (omit if using --last)
        id: Option<String>,
        /// Show the most recent session
        #[arg(long)]
        last: bool,
    },
    /// Delete a session
    Delete {
        /// Session ID (omit if using --last)
        id: Option<String>,
        /// Delete the most recent session
        #[arg(long)]
        last: bool,
    },
    /// Clean up sessions older than N days
    Cleanup {
        /// Maximum age in days (default: 30)
        #[arg(long, default_value = "30")]
        older_than: i64,
    },
}

#[derive(Subcommand)]
enum ServerCommands {
    /// List configured servers
    List,
    /// List running servers
    Running,
    /// Start a server
    Start {
        /// Server name
        name: String,
    },
    /// Stop a server
    Stop {
        /// Server name
        name: String,
    },
    /// List available tools
    Tools,
}

#[derive(Subcommand)]
enum AuditCommands {
    /// List audit sessions
    List,
    /// Show audit entries for a session
    Show {
        /// Session ID
        session_id: String,
    },
    /// Verify audit chain integrity
    Verify {
        /// Session ID (optional, verifies all if not provided)
        session_id: Option<String>,
    },
    /// Show audit statistics
    Stats,
}

#[derive(Subcommand)]
enum HookCommands {
    /// List all hooks
    List,
    /// Enable a hook
    Enable {
        /// Hook name
        name: String,
    },
    /// Disable a hook
    Disable {
        /// Hook name
        name: String,
    },
    /// Show hook details
    Info {
        /// Hook name
        name: String,
    },
    /// Show hook statistics
    Stats,
    /// Test a hook
    Test {
        /// Hook name
        name: String,
        /// Dry run (don't actually execute)
        #[arg(short, long)]
        dry_run: bool,
    },
    /// List available profiles
    Profiles,
}

#[derive(Subcommand)]
enum KeyCommands {
    /// Show the current key (public key and key ID)
    Show,
    /// Generate a new key (prompts if one already exists)
    Generate {
        /// Force overwrite without confirmation
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show resolved configuration with source annotations
    Show {
        /// Output format (toml or json)
        #[arg(short, long, default_value = "toml")]
        format: String,
        /// Show only a specific section (e.g. model, budget, security)
        #[arg(short, long)]
        section: Option<String>,
    },
    /// Validate the current configuration
    Validate,
    /// Show config file paths being checked
    Paths,
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

#[derive(Subcommand)]
enum PluginCommands {
    /// List installed plugins
    List,
    /// Install a plugin from a local path or registry
    Install {
        /// Plugin source (local path or package name)
        source: String,
        /// Install from the `OpenClaw` registry
        #[arg(long)]
        from_openclaw: bool,
        /// Install to workspace instead of user-level
        #[arg(long)]
        workspace: bool,
    },
    /// Remove an installed plugin
    Remove {
        /// Plugin ID
        id: String,
    },
    /// Compile a plugin without loading it
    Compile {
        /// Path to plugin source
        path: String,
        /// Output directory
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Show detailed plugin information
    Info {
        /// Plugin ID
        id: String,
    },
}

/// Ensure the global config directory and `config.toml` exist.
///
/// Returns `true` if this is a first run (config was just created).
fn ensure_global_config() -> bool {
    let Ok(home) = astrid_core::dirs::AstridHome::resolve() else {
        return false;
    };

    if let Err(e) = home.ensure() {
        eprintln!("Warning: could not create ~/.astrid directory: {e}");
        return false;
    }

    let config_path = home.config_path();
    if config_path.exists() {
        return false;
    }

    // Write a commented template so users know what's available.
    let template = r#"# Astrid configuration
# Documentation: https://github.com/astrid-rs/astrid
#
# This file was auto-created on first run. Uncomment and edit as needed.

[model]
# provider = "claude"
# model = "claude-sonnet-4-20250514"
# api_key = ""                # or set ANTHROPIC_API_KEY env var
# max_tokens = 4096
# temperature = 0.7

[runtime]
# max_context_tokens = 180000
# auto_summarize = true

[budget]
# session_max_usd = 5.0
# per_action_max_usd = 0.50

[security.policy]
# require_approval_for_delete = true
# require_approval_for_network = false
"#;

    if let Err(e) = std::fs::write(&config_path, template) {
        eprintln!("Warning: could not create {}: {e}", config_path.display());
        return false;
    }

    true
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
        lc
    } else {
        // Fallback if config loading fails.
        let level = if cli.verbose { "debug" } else { "info" };
        astrid_telemetry::LogConfig::new(level).with_format(astrid_telemetry::LogFormat::Compact)
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
                // JSON mode: print banner to stderr for piping.
                print_banner();
            }
            ensure_global_config();
            if !onboarding::has_api_key() {
                onboarding::run_onboarding();
            }
            onboarding::run_spark_onboarding();
            let workspace = std::env::current_dir().ok();
            chat::run_chat(session, workspace, output_format).await?;
        },
        Some(Commands::Run {
            foreground,
            config: config_path,
        }) => {
            run::run_gateway(foreground, config_path.as_deref()).await?;
        },
        Some(Commands::Daemon { command }) => {
            handle_daemon(command).await?;
        },
        Some(Commands::Doctor) => {
            doctor::run_doctor().await?;
        },
        Some(Commands::Hooks { command }) => {
            handle_hooks(command).await?;
        },
        Some(Commands::Sessions { command }) => {
            handle_sessions(command)?;
        },
        Some(Commands::Servers { command }) => {
            handle_servers(command).await?;
        },
        Some(Commands::Audit { command }) => {
            handle_audit(command)?;
        },
        Some(Commands::Config { command }) => {
            handle_config(command)?;
        },
        Some(Commands::Keys { command }) => {
            handle_keys(&command)?;
        },
        Some(Commands::Capsule { command }) => {
            handle_capsules(command)?;
        },
        Some(Commands::Build { path, output, project_type, from_mcp_json }) => {
            commands::build::run_build(path.as_deref(), output.as_deref(), project_type.as_deref(), from_mcp_json.as_deref())?;
        },
        Some(Commands::Init) => {
            init::run_init()?;
        },
        None => {
            // Default to chat mode.
            if output_format == formatter::OutputFormat::Json {
                print_banner();
            }
            ensure_global_config();
            if !onboarding::has_api_key() {
                onboarding::run_onboarding();
            }
            onboarding::run_spark_onboarding();
            let workspace = std::env::current_dir().ok();
            chat::run_chat(None, workspace, output_format).await?;
        },
    }

    Ok(())
}

async fn handle_daemon(command: DaemonCommands) -> Result<()> {
    match command {
        DaemonCommands::Run {
            ephemeral,
            grace_period,
        } => daemon::run_daemon_with_mode(ephemeral, grace_period).await,
        DaemonCommands::Status => daemon::daemon_status().await,
        DaemonCommands::Stop => daemon::daemon_stop().await,
    }
}

fn handle_sessions(command: SessionCommands) -> Result<()> {
    use astrid_core::dirs::AstridHome;
    use astrid_runtime::SessionStore;

    let home = AstridHome::resolve()?;
    let store = SessionStore::from_home(&home);

    match command {
        SessionCommands::List => sessions::list_sessions(&store),
        SessionCommands::Show { id, last } => {
            let resolved = resolve_session_id(&store, id, last)?;
            sessions::show_session(&store, &resolved)
        },
        SessionCommands::Delete { id, last } => {
            let resolved = resolve_session_id(&store, id, last)?;
            sessions::delete_session(&store, &resolved)
        },
        SessionCommands::Cleanup { older_than } => sessions::cleanup_sessions(&store, older_than),
    }
}

/// Resolve a session ID from either an explicit ID or `--last`.
fn resolve_session_id(
    store: &astrid_runtime::SessionStore,
    id: Option<String>,
    last: bool,
) -> Result<String> {
    match (id, last) {
        (Some(id), false) => Ok(id),
        (None, true) => {
            let session = store
                .most_recent()?
                .ok_or_else(|| anyhow::anyhow!("No sessions found"))?;
            Ok(session.id.0.to_string())
        },
        (Some(_), true) => anyhow::bail!("Cannot specify both a session ID and --last"),
        (None, false) => anyhow::bail!("Provide a session ID or use --last"),
    }
}

async fn handle_servers(command: ServerCommands) -> Result<()> {
    match command {
        ServerCommands::List => {
            // Try daemon first for live status, fall back to static config.
            if let Ok(dc) = crate::daemon_client::DaemonClient::connect().await {
                servers::list_servers_via_daemon(&dc).await?;
            } else {
                let config = astrid_mcp::ServersConfig::load_default().unwrap_or_default();
                servers::list_servers(&config);
            }
            Ok(())
        },
        ServerCommands::Running => {
            let dc = crate::daemon_client::DaemonClient::connect().await?;
            servers::list_servers_via_daemon(&dc).await
        },
        ServerCommands::Start { name } => {
            let dc = crate::daemon_client::DaemonClient::connect().await?;
            servers::start_server(&dc, &name).await
        },
        ServerCommands::Stop { name } => {
            let dc = crate::daemon_client::DaemonClient::connect().await?;
            servers::stop_server(&dc, &name).await
        },
        ServerCommands::Tools => {
            let dc = crate::daemon_client::DaemonClient::connect().await?;
            servers::list_tools(&dc).await
        },
    }
}

fn handle_audit(command: AuditCommands) -> Result<()> {
    use astrid_audit::AuditLog;
    use astrid_core::dirs::AstridHome;
    use astrid_crypto::KeyPair;

    let home = AstridHome::resolve()?;
    home.ensure()?;

    let key = KeyPair::load_or_generate(home.user_key_path())?;
    let log = AuditLog::open(home.audit_db_path(), key)?;

    match command {
        AuditCommands::List => audit::list_audit_sessions(&log),
        AuditCommands::Show { session_id } => audit::show_audit_entries(&log, &session_id),
        AuditCommands::Verify { session_id } => {
            audit::verify_audit_chain(&log, session_id.as_deref())
        },
        AuditCommands::Stats => audit::show_audit_stats(&log),
    }
}

async fn handle_hooks(command: HookCommands) -> Result<()> {
    match command {
        HookCommands::List => hooks::list_hooks(),
        HookCommands::Enable { name } => hooks::enable_hook(&name),
        HookCommands::Disable { name } => hooks::disable_hook(&name),
        HookCommands::Info { name } => hooks::hook_info(&name),
        HookCommands::Stats => return hooks::hook_stats().await,
        HookCommands::Test { name, dry_run } => return hooks::test_hook(&name, dry_run).await,
        HookCommands::Profiles => hooks::list_profiles(),
    }
    Ok(())
}

fn handle_config(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Show { format, section } => {
            config::show_config(&format, section.as_deref())
        },
        ConfigCommands::Validate => config::validate_config(),
        ConfigCommands::Paths => config::show_paths(),
    }
}

fn handle_keys(command: &KeyCommands) -> Result<()> {
    match command {
        KeyCommands::Show => keys::show_key(),
        KeyCommands::Generate { force } => keys::generate_key(*force),
    }
}

fn handle_capsules(command: CapsuleCommands) -> Result<()> {
    match command {
        CapsuleCommands::Install { source, workspace } => {
            capsule::install::install_capsule(&source, workspace)
        },
    }
}
