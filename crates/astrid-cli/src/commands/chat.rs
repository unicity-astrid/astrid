//! Chat command - interactive agent session via daemon.
//!
//! The CLI is a thin client: it connects to the daemon (auto-starting if needed),
//! creates or resumes a session, subscribes to events, and renders output.
//! All heavy lifting (LLM calls, MCP, security) happens in the daemon.

use astrid_core::{
    ApprovalDecision, ApprovalOption, ApprovalRequest, ElicitationRequest, ElicitationResponse,
    ElicitationSchema, SessionId,
};
use astrid_kernel::rpc::DaemonEvent;
use colored::Colorize;
use dialoguer::{Confirm, Input, Password, Select, theme::ColorfulTheme};

use crate::commands::onboarding;
use crate::daemon_client::DaemonClient;
use crate::formatter::{OutputFormat, OutputFormatter, create_formatter};
use crate::repl::ReadlineEvent;
use crate::theme::Theme;

/// Run interactive chat mode via the daemon.
pub(crate) async fn run_chat(
    session_id: Option<String>,
    workspace: Option<std::path::PathBuf>,
    format: OutputFormat,
) -> anyhow::Result<()> {
    // Ensure the user has an API key before connecting to the daemon.
    if !onboarding::has_api_key() {
        onboarding::run_onboarding();
    }

    // Connect to daemon (auto-starts if not running).
    let client = DaemonClient::connect().await?;

    // Create or resume a session.
    let session_info = if let Some(id) = session_id {
        let uuid = uuid::Uuid::parse_str(&id)?;
        client.resume_session(SessionId::from_uuid(uuid)).await?
    } else {
        client.create_session(workspace).await?
    };

    let session_id = session_info.id.clone();

    // Resolve model name from config (best effort).
    let model_name = resolve_model_name();

    match format {
        OutputFormat::Pretty => {
            // TUI mode — replaces the rustyline REPL.
            crate::tui::run(&client, &session_id, &session_info, &model_name).await?;
        },
        OutputFormat::Json => {
            // JSON/NDJSON mode — keep the existing stdin loop.
            run_json_chat(&client, &session_id, &session_info, format).await?;
        },
    }

    Ok(())
}

/// Resolve the model name from configuration.
fn resolve_model_name() -> String {
    let workspace_root = std::env::current_dir().ok();
    astrid_config::Config::load(workspace_root.as_deref())
        .ok()
        .map_or_else(|| "unknown".to_string(), |r| r.config.model.model)
}

/// Run the JSON/NDJSON chat loop (non-TUI mode for piping).
#[allow(clippy::too_many_lines)]
async fn run_json_chat(
    client: &DaemonClient,
    session_id: &SessionId,
    session_info: &astrid_kernel::rpc::SessionInfo,
    format: OutputFormat,
) -> anyhow::Result<()> {
    let mut formatter: Box<dyn OutputFormatter> = create_formatter(format);

    println!(
        "Session: {} | Type {} to quit, {} for help\n",
        Theme::session_id(&session_id.0.to_string()),
        "exit".cyan(),
        "/help".cyan()
    );

    if session_info.pending_deferred_count > 0 {
        let n = session_info.pending_deferred_count;
        let s = if n == 1 { "" } else { "s" };
        let v = if n == 1 { "s" } else { "" };
        println!(
            "{}",
            Theme::warning(&format!("{n} deferred item{s} need{v} your attention"))
        );
    }

    let mut event_sub = client.subscribe_events(session_id).await?;
    let mut editor = crate::repl::ReplEditor::new()?;

    loop {
        let input = match editor.readline() {
            ReadlineEvent::Line(line) => line,
            ReadlineEvent::Interrupted => continue,
            ReadlineEvent::Eof => {
                println!("{}", Theme::dimmed("Goodbye!"));
                break;
            },
        };

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if input == "exit" || input == "quit" {
            println!("{}", Theme::dimmed("Goodbye!"));
            break;
        }

        if input.starts_with('/') {
            handle_slash_command(input, client, session_id).await;
            continue;
        }

        client.send_input(session_id, input).await?;

        loop {
            let Some(event_result) = event_sub.next().await else {
                eprintln!("{}", Theme::error("Connection to daemon lost"));
                return Ok(());
            };

            let event = match event_result {
                Ok(event) => event,
                Err(e) => {
                    eprintln!(
                        "{}",
                        Theme::warning(&format!("Event deserialization error: {e}"))
                    );
                    continue;
                },
            };

            match event {
                DaemonEvent::Text(ref text) => formatter.format_text(text),
                DaemonEvent::ToolCallStart {
                    ref id,
                    ref name,
                    ref args,
                } => {
                    formatter.flush_markdown();
                    formatter.format_tool_start(id, name, args);
                },
                DaemonEvent::ToolCallResult {
                    ref id,
                    ref result,
                    is_error,
                } => {
                    formatter.format_tool_result(id, result, is_error);
                },
                DaemonEvent::ApprovalNeeded {
                    request_id,
                    request,
                } => {
                    formatter.flush_markdown();
                    let decision = handle_approval_prompt(&request, client, session_id).await?;
                    client
                        .send_approval(session_id, &request_id, decision)
                        .await?;
                },
                DaemonEvent::ElicitationNeeded {
                    request_id,
                    request,
                } => {
                    formatter.flush_markdown();
                    let response = handle_elicitation_prompt(&request)?;
                    client
                        .send_elicitation(session_id, &request_id, response)
                        .await?;
                },
                DaemonEvent::CapsuleLoaded { ref name, .. } => {
                    println!("{}", Theme::success(&format!("Plugin loaded: {name}")));
                },
                DaemonEvent::CapsuleFailed { ref id, ref error } => {
                    println!(
                        "{}",
                        Theme::warning(&format!("Plugin {id} failed to load: {error}"))
                    );
                },
                DaemonEvent::CapsuleUnloaded { ref name, .. } => {
                    println!("{}", Theme::dimmed(&format!("Plugin unloaded: {name}")));
                },
                DaemonEvent::Usage { .. } | DaemonEvent::SessionSaved => {},
                DaemonEvent::TurnComplete => {
                    formatter.format_turn_complete();
                    break;
                },
                DaemonEvent::Error(ref msg) => {
                    formatter.flush_markdown();
                    formatter.format_error(msg);
                },
            }
        }
    }

    if let Err(e) = client.end_session(session_id).await {
        eprintln!(
            "{}",
            Theme::warning(&format!("Failed to end session cleanly: {e}"))
        );
    }

    Ok(())
}

// ─── Slash Commands (JSON mode) ──────────────────────────────────

/// Handle slash commands (JSON mode — prints to stdout).
#[allow(clippy::too_many_lines)]
async fn handle_slash_command(command: &str, client: &DaemonClient, session_id: &SessionId) {
    use std::io::{self, Write};

    let parts: Vec<&str> = command.split_whitespace().collect();
    let cmd = parts.first().copied().unwrap_or("");
    let arg = parts.get(1).copied();

    match cmd {
        "/help" => print_help(),
        "/clear" => {
            print!("\x1B[2J\x1B[1;1H");
            let _ = io::stdout().flush();
        },
        "/info" => {
            if let Ok(status) = client.status().await {
                println!("\n{}", Theme::header("Daemon Info"));
                println!("  Version:  {}", status.version);
                println!("  Uptime:   {}s", status.uptime_secs);
                println!("  Sessions: {}", status.active_sessions);
                println!(
                    "  MCP:      {}/{} servers running",
                    status.mcp_servers_running, status.mcp_servers_configured
                );
                println!("  Plugins:  {} loaded\n", status.capsules_loaded);
            } else {
                println!("{}", Theme::error("Failed to get daemon info"));
            }
        },
        "/servers" => match client.list_servers().await {
            Ok(servers) if servers.is_empty() => {
                println!("\n{}", Theme::dimmed("No MCP servers configured.\n"));
            },
            Ok(servers) => {
                println!("\n{}", Theme::header("MCP Servers"));
                for s in &servers {
                    let status_icon = if s.ready {
                        "●".green().to_string()
                    } else if s.alive {
                        "○".yellow().to_string()
                    } else {
                        "○".red().to_string()
                    };
                    let desc = s.description.as_deref().unwrap_or("");
                    println!(
                        "  {} {} ({} tools) {}",
                        status_icon,
                        s.name,
                        s.tool_count,
                        desc.dimmed()
                    );
                }
                println!();
            },
            Err(e) => println!("{}", Theme::error(&format!("Failed to list servers: {e}"))),
        },
        "/tools" => match client.list_tools().await {
            Ok(tools) if tools.is_empty() => {
                println!("\n{}", Theme::dimmed("No tools available.\n"));
            },
            Ok(tools) => {
                let filtered: Vec<_> = if let Some(server_filter) = arg {
                    tools.iter().filter(|t| t.server == server_filter).collect()
                } else {
                    tools.iter().collect()
                };
                println!("\n{}", Theme::header("Available Tools"));
                let mut current_server = "";
                for t in &filtered {
                    if t.server != current_server {
                        current_server = &t.server;
                        println!("  {}", current_server.bold());
                    }
                    let desc = t.description.as_deref().unwrap_or("");
                    println!("    {} {}", t.name.cyan(), desc.dimmed());
                }
                println!();
            },
            Err(e) => println!("{}", Theme::error(&format!("Failed to list tools: {e}"))),
        },
        "/allowances" => match client.session_allowances(session_id).await {
            Ok(allowances) if allowances.is_empty() => {
                println!("\n{}", Theme::dimmed("No active allowances.\n"));
            },
            Ok(allowances) => {
                println!("\n{}", Theme::header("Active Allowances"));
                for a in &allowances {
                    let scope = if a.session_only {
                        "session".yellow().to_string()
                    } else {
                        "workspace".cyan().to_string()
                    };
                    let uses = a
                        .uses_remaining
                        .map_or_else(|| "unlimited".to_string(), |n| format!("{n} uses left"));
                    println!("  [{scope}] {} ({uses})", a.pattern);
                }
                println!();
            },
            Err(e) => println!(
                "{}",
                Theme::error(&format!("Failed to get allowances: {e}"))
            ),
        },
        "/budget" => match client.session_budget(session_id).await {
            Ok(budget) => {
                println!("\n{}", Theme::header("Budget"));
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pct = if budget.session_max_usd > 0.0 {
                    (budget.session_spent_usd / budget.session_max_usd * 100.0).clamp(0.0, 100.0)
                        as u8
                } else {
                    0
                };
                let bar = budget_bar(pct);
                println!(
                    "  Session: ${:.4} / ${:.2} ({pct}%) {bar}",
                    budget.session_spent_usd, budget.session_max_usd,
                );
                println!("  Per-action limit: ${:.2}", budget.per_action_max_usd);
                if let Some(ws_spent) = budget.workspace_spent_usd {
                    let ws_max = budget
                        .workspace_max_usd
                        .map_or_else(|| "unlimited".to_string(), |m| format!("${m:.2}"));
                    println!("  Workspace: ${ws_spent:.4} / {ws_max}");
                }
                println!();
            },
            Err(e) => println!("{}", Theme::error(&format!("Failed to get budget: {e}"))),
        },
        "/audit" => {
            let limit: Option<usize> = arg.and_then(|a| a.parse().ok());
            match client.session_audit(session_id, limit).await {
                Ok(entries) if entries.is_empty() => {
                    println!("\n{}", Theme::dimmed("No audit entries.\n"));
                },
                Ok(entries) => {
                    println!("\n{}", Theme::header("Recent Audit Entries"));
                    for e in &entries {
                        println!(
                            "  {} {} {}",
                            Theme::timestamp(&e.timestamp),
                            e.action,
                            e.outcome.dimmed(),
                        );
                    }
                    println!();
                },
                Err(e) => println!("{}", Theme::error(&format!("Failed to get audit log: {e}"))),
            }
        },
        "/save" => match client.save_session(session_id).await {
            Ok(()) => println!("{}", Theme::success("Session saved.")),
            Err(e) => println!("{}", Theme::error(&format!("Failed to save: {e}"))),
        },
        "/plugins" => match client.list_capsules().await {
            Ok(plugins) if plugins.is_empty() => {
                println!("\n{}", Theme::dimmed("No plugins registered.\n"));
            },
            Ok(plugins) => {
                println!("\n{}", Theme::header("Plugins"));
                for p in &plugins {
                    let status_icon = match p.state.as_str() {
                        "ready" => "●".green().to_string(),
                        "loading" | "unloading" => "○".yellow().to_string(),
                        "failed" => "●".red().to_string(),
                        _ => "○".dimmed().to_string(),
                    };
                    let error_hint = p
                        .error
                        .as_deref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default();
                    let desc = p.description.as_deref().unwrap_or("");
                    println!(
                        "  {} {} v{} [{}] ({} tools) {} {}",
                        status_icon,
                        p.name,
                        p.version,
                        p.state,
                        p.tool_count,
                        desc.dimmed(),
                        error_hint.red(),
                    );
                }
                println!();
            },
            Err(e) => println!("{}", Theme::error(&format!("Failed to list plugins: {e}"))),
        },
        "/sessions" => match client.list_sessions(None).await {
            Ok(sessions) if sessions.is_empty() => {
                println!("\n{}", Theme::dimmed("No active sessions.\n"));
            },
            Ok(sessions) => {
                println!("\n{}", Theme::header("Active Sessions"));
                for s in &sessions {
                    let current = if s.id == *session_id {
                        " (current)"
                    } else {
                        ""
                    };
                    println!(
                        "  {} {} | {} msgs{}",
                        Theme::session_id(&s.id.0.to_string()),
                        Theme::timestamp(&s.created_at),
                        s.message_count,
                        current.cyan(),
                    );
                }
                println!();
            },
            Err(e) => println!("{}", Theme::error(&format!("Failed to list sessions: {e}"))),
        },
        _ => {
            println!(
                "{}",
                Theme::warning(&format!(
                    "Unknown command: {cmd}. Type /help for available commands."
                ))
            );
        },
    }
}

fn print_help() {
    println!("\n{}", Theme::header("Available Commands"));
    println!("  {}   Show this help", "/help".cyan());
    println!("  {}  Clear the screen", "/clear".cyan());
    println!("  {}   Show daemon info", "/info".cyan());
    println!("  {}", "/servers".cyan());
    println!("         List connected MCP servers");
    println!("  {}", "/tools [server]".cyan());
    println!("         List available tools (optionally filter by server)");
    println!("  {}", "/allowances".cyan());
    println!("         Show active session allowances");
    println!("  {}", "/budget".cyan());
    println!("         Show budget usage and remaining");
    println!("  {}", "/plugins".cyan());
    println!("         List registered plugins and their status");
    println!("  {}", "/audit [N]".cyan());
    println!("         Show last N audit entries (default: 20)");
    println!("  {}   Save session explicitly", "/save".cyan());
    println!("  {}", "/sessions".cyan());
    println!("         List active sessions");
    println!("  {}   Exit the chat", "exit".cyan());
    println!();
}

fn budget_bar(percent: u8) -> String {
    // .min(100) / 5 guarantees filled ∈ [0, 20]
    let filled = (percent as usize).min(100) / 5;
    let empty = 20usize.saturating_sub(filled);
    let bar = format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));
    if percent >= 90 {
        bar.red().to_string()
    } else if percent >= 70 {
        bar.yellow().to_string()
    } else {
        bar.green().to_string()
    }
}

// ─── Approval / Elicitation Prompts (JSON mode) ──────────────────

async fn handle_approval_prompt(
    request: &ApprovalRequest,
    client: &DaemonClient,
    session_id: &SessionId,
) -> anyhow::Result<ApprovalDecision> {
    let mut content_lines = Vec::new();
    content_lines.push(Theme::kv("Action", &request.operation));
    content_lines.push(format!("  {}", request.description));
    if let Some(ref resource) = request.resource {
        content_lines.push(Theme::kv("Resource", resource));
    }
    content_lines.push(Theme::kv("Risk", &Theme::risk_level(request.risk_level)));

    if let Ok(budget) = client.session_budget(session_id).await {
        content_lines.push(Theme::kv(
            "Budget",
            &format!(
                "${:.4} / ${:.2} remaining",
                budget.session_spent_usd, budget.session_max_usd
            ),
        ));
    }

    let content = content_lines.join("\n");
    println!(
        "\n{}",
        Theme::approval_box("Approval Required", &content, request.risk_level)
    );

    let options: Vec<String> = request.options.iter().map(ToString::to_string).collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .items(&options)
        .default(0)
        .interact()?;

    let decision_option = request.options[selection];

    let reason = if decision_option == ApprovalOption::Deny {
        let r = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Reason (optional)")
            .allow_empty(true)
            .interact_text()
            .ok();
        r.filter(|s| !s.is_empty())
    } else {
        None
    };

    let mut approval = ApprovalDecision::new(request.request_id, decision_option);
    if let Some(r) = reason {
        approval = approval.with_reason(r);
    }

    Ok(approval)
}

fn handle_elicitation_prompt(request: &ElicitationRequest) -> anyhow::Result<ElicitationResponse> {
    println!("\n{}", Theme::separator());
    println!("{}", Theme::header("Input Required"));
    println!("From: {}", request.server_name.cyan());
    println!("{}", request.message);
    println!("{}", Theme::separator());

    let theme = ColorfulTheme::default();

    let value = match &request.schema {
        ElicitationSchema::Text {
            placeholder,
            max_length,
        } => {
            let prompt = placeholder
                .clone()
                .unwrap_or_else(|| "Enter value".to_string());
            let input = Input::<String>::with_theme(&theme)
                .with_prompt(&prompt)
                .allow_empty(!request.required);

            let text = input.interact_text()?;

            if let Some(max) = max_length
                && text.len() > *max
            {
                anyhow::bail!("Input exceeds max length of {max}");
            }

            serde_json::Value::String(text)
        },
        ElicitationSchema::Secret { placeholder } => {
            let prompt = placeholder
                .clone()
                .unwrap_or_else(|| "Enter secret".to_string());
            let secret = Password::with_theme(&theme)
                .with_prompt(&prompt)
                .interact()?;
            serde_json::Value::String(secret)
        },
        ElicitationSchema::Select { options, multiple } => {
            let labels: Vec<_> = options.iter().map(|o| &o.label).collect();

            if *multiple {
                let selections = dialoguer::MultiSelect::with_theme(&theme)
                    .items(&labels)
                    .interact()?;

                let values: Vec<_> = selections
                    .iter()
                    .map(|&i| serde_json::Value::String(options[i].value.clone()))
                    .collect();

                serde_json::Value::Array(values)
            } else {
                let selection = Select::with_theme(&theme)
                    .items(&labels)
                    .default(0)
                    .interact()?;

                serde_json::Value::String(options[selection].value.clone())
            }
        },
        ElicitationSchema::Confirm { default } => {
            let confirmed = Confirm::with_theme(&theme)
                .with_prompt("Confirm?")
                .default(*default)
                .interact()?;

            serde_json::Value::Bool(confirmed)
        },
    };

    Ok(ElicitationResponse::submit(request.request_id, value))
}
