//! Chat command - interactive agent session via daemon.
//!
//! The CLI is a thin client: it connects to the daemon (auto-starting if needed),
//! creates or resumes a session, subscribes to events, and renders output.
//! All heavy lifting (LLM calls, MCP, security) happens in the daemon.

use astrid_core::SessionId;
use colored::Colorize;

use crate::formatter::{OutputFormat, OutputFormatter, create_formatter};
use crate::repl::ReadlineEvent;
use crate::socket_client::SocketClient;
use crate::theme::Theme;

/// Reason sent (and displayed) when the JSON REPL auto-denies an approval request.
const APPROVAL_UNSUPPORTED_REASON: &str = "approvals not supported in JSON REPL mode";

/// Run interactive chat mode via the daemon.
pub(crate) async fn run_chat(
    client: &mut SocketClient,
    session_id: &SessionId,
    model_name: &str,
    format: OutputFormat,
) -> anyhow::Result<()> {
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
            handle_slash_command(input, client, session_id);
            continue;
        }

        client.send_input(input.to_string()).await?;

        loop {
            let Some(event) = client.read_event().await? else {
                eprintln!("{}", Theme::error("Connection to daemon lost"));
                return Ok(());
            };

            let astrid_events::AstridEvent::Ipc { message, .. } = event else {
                continue;
            };

            match message.payload {
                astrid_events::ipc::IpcPayload::AgentResponse { text, is_final, .. } => {
                    formatter.format_text(&text);
                    if is_final {
                        formatter.flush_markdown();
                        break;
                    }
                },
                astrid_events::ipc::IpcPayload::LlmStreamEvent {
                    event: astrid_events::llm::StreamEvent::ToolCallStart { id, name },
                    ..
                } => {
                    formatter.flush_markdown();
                    formatter.format_tool_start(&id, &name, &serde_json::Value::Null);
                },
                astrid_events::ipc::IpcPayload::ToolExecuteResult { call_id, result } => {
                    formatter.flush_markdown();
                    let res_val = serde_json::to_string(&result.content).unwrap_or_default();
                    formatter.format_tool_result(&call_id, &res_val, result.is_error);
                },
                astrid_events::ipc::IpcPayload::ApprovalRequired {
                    request_id,
                    action,
                    resource,
                    reason,
                    risk_level,
                } => {
                    formatter.flush_markdown();
                    println!(
                        "{}",
                        Theme::warning(&format!(
                            "Approval required [{risk_level}]: {action} on {resource} ({reason})"
                        ))
                    );
                    // JSON REPL auto-denies - TUI handles interactive approval
                    client
                        .send_message(astrid_events::ipc::IpcMessage::new(
                            format!("astrid.v1.approval.response.{request_id}"),
                            astrid_events::ipc::IpcPayload::ApprovalResponse {
                                request_id,
                                decision: "deny".into(),
                                reason: Some(APPROVAL_UNSUPPORTED_REASON.into()),
                            },
                            session_id.0,
                        ))
                        .await?;
                    println!(
                        "{}",
                        Theme::dimmed(&format!("Auto-denied: {APPROVAL_UNSUPPORTED_REASON}"))
                    );
                },
                _ => {}, // Ignore other IPC payloads for now
            }
        }
    }

    Ok(())
}

fn handle_slash_command(cmd: &str, _client: &mut SocketClient, _session_id: &SessionId) {
    println!("Slash commands temporarily disabled in JSON Mode during microkernel refactor: {cmd}");
}
