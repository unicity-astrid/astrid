//! Chat command - interactive agent session via daemon.
//!
//! The CLI is a thin client: it connects to the daemon (auto-starting if needed),
//! creates or resumes a session, subscribes to events, and renders output.
//! All heavy lifting (LLM calls, MCP, security) happens in the daemon.

use astrid_core::SessionId;
use colored::Colorize;

use crate::commands::onboarding;
use crate::socket_client::SocketClient;
use crate::formatter::{OutputFormat, OutputFormatter, create_formatter};
use crate::repl::ReadlineEvent;
use crate::theme::Theme;

/// Run interactive chat mode via the daemon.
pub(crate) async fn run_chat(
    client: &mut SocketClient,
    session_id: &SessionId,
    model_name: &str,
    format: OutputFormat,
) -> anyhow::Result<()> {
    // Ensure the user has an API key before connecting to the daemon.
    if !onboarding::has_api_key() {
        onboarding::run_onboarding();
    }

    match format {
        OutputFormat::Pretty => {
            // TUI mode — replaces the rustyline REPL.
            let workspace = std::env::current_dir().ok();
            crate::tui::run(client, session_id, workspace, model_name).await?;
        },
        OutputFormat::Json => {
            // JSON/NDJSON mode — keep the existing stdin loop.
            run_json_chat(client, session_id, format).await?;
        },
    }

    Ok(())
}

async fn run_json_chat(
    client: &mut SocketClient,
    session_id: &SessionId,
    format: OutputFormat,
) -> anyhow::Result<()> {
    let mut formatter: Box<dyn OutputFormatter> = create_formatter(format);

    println!(
        "Session: {} | Type {} to quit, {} for help
",
        Theme::session_id(&session_id.0.to_string()),
        "exit".cyan(),
        "/help".cyan()
    );

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

        client.send_input(input.to_string()).await?;

        loop {
            let Some(event) = client.read_event().await? else {
                eprintln!("{}", Theme::error("Connection to daemon lost"));
                return Ok(());
            };

            let astrid_events::AstridEvent::Ipc { message, .. } = event else { continue };

            match message.payload {
                astrid_events::ipc::IpcPayload::AgentResponse { text, is_final } => {
                    formatter.format_text(&text);
                    if is_final {
                        formatter.flush_markdown();
                        break;
                    }
                },
                astrid_events::ipc::IpcPayload::ToolCallStart { id, name, args } => {
                    formatter.flush_markdown();
                    let args_val = serde_json::to_value(&args).unwrap_or_default();
                    formatter.format_tool_start(&id.to_string(), &name, &args_val);
                },
                astrid_events::ipc::IpcPayload::ToolCallResult {
                    id,
                    result,
                    is_error,
                } => {
                    formatter.flush_markdown();
                    let res_val = serde_json::to_value(&result).unwrap_or_default();
                    formatter.format_tool_result(&id.to_string(), &res_val, is_error);
                },
                astrid_events::ipc::IpcPayload::ApprovalRequired {
                    action,
                    resource,
                    reason,
                } => {
                    formatter.flush_markdown();
                    println!("{}", Theme::warning(&format!("Approval required: {} on {} ({})", action, resource, reason)));
                    // TUI handles this correctly, JSON mode auto-aborts for safety unless headless approvals are enabled
                    client.send_message(astrid_events::ipc::IpcMessage::new(
                        "user.approval",
                        astrid_events::ipc::IpcPayload::RawJson(serde_json::json!({
                            "approved": false,
                            "reason": "Approval not supported in JSON REPL mode"
                        })),
                        session_id.0
                    )).await?;
                },
                astrid_events::ipc::IpcPayload::ElicitationRequired {
                    action,
                    resource,
                    ..
                } => {
                    formatter.flush_markdown();
                    println!("{}", Theme::warning(&format!("Elicitation required: {} on {}", action, resource)));
                    // Same as approval, auto-cancel in non-interactive REPL mode
                     client.send_message(astrid_events::ipc::IpcMessage::new(
                        "user.elicitation",
                        astrid_events::ipc::IpcPayload::RawJson(serde_json::json!({
                            "cancelled": true,
                            "reason": "Elicitation not supported in JSON REPL mode"
                        })),
                        session_id.0
                    )).await?;
                },
                _ => {} // Ignore other IPC payloads for now
            }
        }
    }

    Ok(())
}

async fn handle_slash_command(
    cmd: &str,
    _client: &mut SocketClient,
    _session_id: &SessionId,
) {
    println!("Slash commands temporarily disabled in JSON Mode during microkernel refactor: {}", cmd);
}