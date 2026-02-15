//! TUI module — ratatui-based terminal interface.
//!
//! Connects the Nexus view to the real daemon via `DaemonClient`.

mod input;
mod render;
pub(crate) mod state;
mod theme;

use std::fmt::Write as _;
use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use astralis_core::{
    ApprovalDecision, ApprovalOption, ElicitationResponse, RiskLevel as CoreRiskLevel, SessionId,
};
use astralis_gateway::rpc::{DaemonEvent, SessionInfo};
use crossterm::{
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::daemon_client::DaemonClient;
use state::{
    App, ApprovalDecisionKind, ApprovalRequest, Message, MessageKind, MessageRole, NexusEntry,
    PendingAction, RiskLevel, ToolStatus, ToolStatusKind, UiState,
};

/// Type alias for our terminal.
type Term = Terminal<CrosstermBackend<Stdout>>;

/// Initialize the terminal for TUI mode.
///
/// Returns the terminal and whether keyboard enhancement was enabled.
/// Keyboard enhancement (Kitty protocol) provides unambiguous key events,
/// preventing escape sequence bytes from leaking as character input on
/// terminals like Alacritty that support the protocol.
fn init_terminal() -> io::Result<(Term, bool)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();

    let keyboard_enhanced = matches!(supports_keyboard_enhancement(), Ok(true));

    if keyboard_enhanced {
        execute!(
            stdout,
            EnterAlternateScreen,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    } else {
        execute!(stdout, EnterAlternateScreen)?;
    }

    let backend = CrosstermBackend::new(stdout);
    Ok((Terminal::new(backend)?, keyboard_enhanced))
}

/// Restore terminal to normal mode.
fn restore_terminal(terminal: &mut Term, keyboard_enhanced: bool) -> io::Result<()> {
    disable_raw_mode()?;
    if keyboard_enhanced {
        execute!(
            terminal.backend_mut(),
            PopKeyboardEnhancementFlags,
            LeaveAlternateScreen
        )?;
    } else {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    }
    terminal.show_cursor()?;
    Ok(())
}

/// Main TUI entry point — replaces the rustyline REPL for Pretty mode.
#[allow(clippy::too_many_lines)]
pub(crate) async fn run(
    client: &DaemonClient,
    session_id: &SessionId,
    session_info: &SessionInfo,
    model_name: &str,
) -> anyhow::Result<()> {
    let working_dir = session_info
        .workspace
        .as_ref()
        .map_or_else(|| "no workspace".to_string(), |p| p.display().to_string());
    let session_id_short = session_id.0.to_string()[..8].to_string();

    let mut app = App::new(working_dir, model_name.to_string(), session_id_short);

    // Surface pending deferred items on session resume.
    if session_info.pending_deferred_count > 0 {
        let n = session_info.pending_deferred_count;
        let s = if n == 1 { "" } else { "s" };
        let v = if n == 1 { "s" } else { "" };
        app.push_notice(&format!("{n} deferred item{s} need{v} your attention"));
    }

    // Subscribe to events before entering the loop.
    let mut event_sub = client.subscribe_events(session_id).await?;

    // Initialize terminal — wrapped in a guard for proper cleanup.
    let (mut terminal, keyboard_enhanced) = init_terminal()?;
    let result = run_loop(&mut terminal, &mut app, client, session_id, &mut event_sub).await;

    // Always restore terminal, even on error.
    let _ = restore_terminal(&mut terminal, keyboard_enhanced);

    result
}

/// Inner run loop — separated so terminal restore always happens.
#[allow(clippy::too_many_lines)]
async fn run_loop(
    terminal: &mut Term,
    app: &mut App,
    client: &DaemonClient,
    session_id: &SessionId,
    event_sub: &mut jsonrpsee::core::client::Subscription<DaemonEvent>,
) -> anyhow::Result<()> {
    let render_interval = Duration::from_millis(16);
    let mut last_render = Instant::now();

    loop {
        // Render if enough time has passed.
        if last_render.elapsed() >= render_interval {
            terminal.draw(|frame| render::render_frame(frame, app))?;
            last_render = Instant::now();
        }

        // Process pending actions (approval decisions, input sends).
        handle_pending_actions(app, client, session_id).await?;

        // Poll for crossterm input events (non-blocking).
        if crossterm::event::poll(Duration::from_millis(10))? {
            input::handle_input(app)?;
        }

        // Poll for daemon events (non-blocking via timeout).
        match tokio::time::timeout(Duration::from_millis(1), event_sub.next()).await {
            Ok(Some(Ok(event))) => {
                handle_daemon_event(app, event);
            },
            Ok(Some(Err(e))) => {
                app.push_notice(&format!("Event error: {e}"));
            },
            Ok(None) => {
                // Subscription closed.
                app.push_notice("Connection to daemon lost.");
                app.state = UiState::Error {
                    message: "Connection to daemon lost".to_string(),
                };
            },
            Err(_) => {
                // Timeout — no event this tick, continue.
            },
        }

        // Update thinking animation dots.
        if let UiState::Thinking { start_time, dots } = &app.state {
            let elapsed = start_time.elapsed();
            let new_dots = ((elapsed.as_millis() / 500) % 4) as usize;
            if new_dots != *dots {
                app.state = UiState::Thinking {
                    start_time: *start_time,
                    dots: new_dots,
                };
            }
        }

        if app.should_quit {
            // Clean disconnect.
            if let Err(e) = client.end_session(session_id).await {
                tracing::warn!("Failed to end session cleanly: {e}");
            }
            break;
        }
    }

    Ok(())
}

/// Map a `DaemonEvent` to TUI state changes.
#[allow(clippy::too_many_lines)]
fn handle_daemon_event(app: &mut App, event: DaemonEvent) {
    // When interrupted, ignore streaming events — only let TurnComplete
    // (and harmless metadata events) through so the UI settles to Idle.
    if app.state == UiState::Interrupted {
        match event {
            DaemonEvent::TurnComplete => {
                app.tokens_streamed = 0;
                app.state = UiState::Idle;
                app.scroll_offset = 0;
            },
            DaemonEvent::Usage {
                context_tokens,
                max_context_tokens,
            } =>
            {
                #[allow(clippy::cast_precision_loss)]
                if max_context_tokens > 0 {
                    app.context_usage =
                        (context_tokens as f32 / max_context_tokens as f32).clamp(0.0, 1.0);
                }
            },
            DaemonEvent::PluginLoaded { name, .. } => {
                app.push_notice(&format!("Plugin loaded: {name}"));
            },
            DaemonEvent::PluginFailed { id, error } => {
                app.push_notice(&format!("Plugin {id} failed: {error}"));
            },
            DaemonEvent::PluginUnloaded { name, .. } => {
                app.push_notice(&format!("Plugin unloaded: {name}"));
            },
            _ => {},
        }
        return;
    }

    match event {
        DaemonEvent::Text(text) => {
            app.stream_buffer.push_str(&text);
            app.tokens_streamed = app
                .tokens_streamed
                .saturating_add(text.split_whitespace().count());

            // Update or create the assistant message in the nexus stream.
            if let Some(NexusEntry::Message(last)) = app.nexus_stream.last_mut()
                && last.role == MessageRole::Assistant
                && last.kind.is_none()
            {
                last.content.push_str(&text);
            } else {
                // Flush previous content and start new assistant message.
                let msg = Message {
                    role: MessageRole::Assistant,
                    content: text,
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                };
                app.nexus_stream.push(NexusEntry::Message(msg));
            }

            // Also keep `messages` in sync for the last assistant message.
            if let Some(last) = app.messages.last_mut()
                && last.role == MessageRole::Assistant
                && last.kind.is_none()
            {
                last.content.push_str(&app.stream_buffer);
                app.stream_buffer.clear();
            } else if !app.stream_buffer.is_empty() {
                app.messages.push(Message {
                    role: MessageRole::Assistant,
                    content: std::mem::take(&mut app.stream_buffer),
                    timestamp: Instant::now(),
                    kind: None,
                    spacing: true,
                });
            }

            if !matches!(app.state, UiState::Streaming { .. }) {
                app.state = UiState::Streaming {
                    start_time: Instant::now(),
                };
            }
        },
        DaemonEvent::ToolCallStart { id, name, args } => {
            // Flush any streaming text.
            flush_stream_buffer(app);

            // Extract a clean display name from the raw tool name.
            // Raw names can be "workspace-escape:builtin:write_file",
            // "server:tool_name", etc. — extract only the final tool name.
            let display_name = clean_tool_name(&name);

            // Extract a display argument from the tool args.
            let display_arg = args
                .as_object()
                .and_then(|o| {
                    o.get("path")
                        .or_else(|| o.get("file_path"))
                        .or_else(|| o.get("command"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();

            app.running_tools.push(ToolStatus {
                id: id.clone(),
                name: display_name.clone(),
                display_arg,
                status: ToolStatusKind::Running,
                start_time: Instant::now(),
                end_time: None,
                output: None,
                expanded: false,
            });

            app.state = UiState::ToolRunning {
                tool_name: display_name,
                start_time: Instant::now(),
            };
        },
        DaemonEvent::ToolCallResult {
            id,
            result,
            is_error,
        } => {
            // Find the running tool by its call ID and move it to completed.
            let pos = app.running_tools.iter().position(|t| t.id == id);
            let tool_entry = if let Some(idx) = pos {
                Some(app.running_tools.remove(idx))
            } else {
                // Fallback: pop the last running tool if ID doesn't match
                // (e.g. approval-inserted tools that lack the original ID).
                app.running_tools.pop()
            };

            if let Some(mut tool) = tool_entry {
                tool.end_time = Some(Instant::now());
                tool.status = if is_error {
                    ToolStatusKind::Failed(result.clone())
                } else {
                    ToolStatusKind::Success
                };
                tool.output = Some(result);
                let idx = app.completed_tools.len();
                app.completed_tools.push(tool);

                // Add an inline tool result to the nexus stream.
                let msg = Message {
                    role: MessageRole::System,
                    content: String::new(),
                    timestamp: Instant::now(),
                    kind: Some(MessageKind::ToolResult(idx)),
                    spacing: true,
                };
                app.nexus_stream.push(NexusEntry::Message(msg.clone()));
                app.messages.push(msg);
            }

            // If more tools are running, stay in ToolRunning; else go Thinking.
            if let Some(tool) = app.running_tools.last() {
                app.state = UiState::ToolRunning {
                    tool_name: tool.name.clone(),
                    start_time: tool.start_time,
                };
            } else {
                app.state = UiState::Thinking {
                    start_time: Instant::now(),
                    dots: 0,
                };
            }
        },
        DaemonEvent::ApprovalNeeded {
            request_id,
            request,
        } => {
            flush_stream_buffer(app);

            // Convert core ApprovalRequest to TUI's local type.
            let risk = match request.risk_level {
                CoreRiskLevel::Low => RiskLevel::Low,
                CoreRiskLevel::Medium => RiskLevel::Medium,
                CoreRiskLevel::High | CoreRiskLevel::Critical => RiskLevel::High,
            };

            let mut details = Vec::new();
            details.push(("Operation".to_string(), request.operation.clone()));
            if let Some(ref resource) = request.resource {
                details.push(("Resource".to_string(), resource.clone()));
            }

            app.pending_approvals.push(ApprovalRequest {
                id: request_id,
                tool_name: request.operation,
                description: request.description,
                risk_level: risk,
                details,
            });
            app.selected_approval = 0;
            app.state = UiState::AwaitingApproval;
        },
        DaemonEvent::ElicitationNeeded {
            request_id,
            request,
        } => {
            // Elicitation not supported in TUI raw mode yet — auto-cancel.
            app.push_notice(&format!(
                "Elicitation from {}: {} (not supported in TUI yet, cancelled)",
                request.server_name, request.message
            ));

            // Queue a cancel response via the pending actions mechanism.
            // The run loop will send an ElicitationResponse::cancel to the daemon.
            app.pending_actions.push(PendingAction::Deny {
                request_id,
                reason: Some("__elicitation_cancel__".to_string()),
            });
        },
        DaemonEvent::Usage {
            context_tokens,
            max_context_tokens,
        } =>
        {
            #[allow(clippy::cast_precision_loss)]
            if max_context_tokens > 0 {
                app.context_usage =
                    (context_tokens as f32 / max_context_tokens as f32).clamp(0.0, 1.0);
            }
        },
        DaemonEvent::SessionSaved => {
            // Silent.
        },
        DaemonEvent::TurnComplete => {
            flush_stream_buffer(app);

            // Record completion for fade-out activity display.
            if let UiState::Thinking { start_time, .. }
            | UiState::Streaming { start_time }
            | UiState::ToolRunning { start_time, .. } = &app.state
            {
                let duration = start_time.elapsed();
                let past_verb = match &app.state {
                    UiState::Thinking { .. } => "Thought",
                    UiState::Streaming { .. } => "Responded",
                    UiState::ToolRunning { tool_name, .. } => {
                        // Borrow issue: just use "Ran tool"
                        let _ = tool_name;
                        "Ran tool"
                    },
                    _ => "Completed",
                };
                app.last_completed = Some((past_verb.to_string(), duration));
                app.last_completed_at = Some(Instant::now());
            }

            app.tokens_streamed = 0;
            app.state = UiState::Idle;
            app.scroll_offset = 0;
        },
        DaemonEvent::Error(msg) => {
            flush_stream_buffer(app);
            app.push_notice(&format!("Error: {msg}"));
            app.state = UiState::Error { message: msg };
        },
        DaemonEvent::PluginLoaded { name, .. } => {
            app.push_notice(&format!("Plugin loaded: {name}"));
        },
        DaemonEvent::PluginFailed { id, error } => {
            app.push_notice(&format!("Plugin {id} failed: {error}"));
        },
        DaemonEvent::PluginUnloaded { name, .. } => {
            app.push_notice(&format!("Plugin unloaded: {name}"));
        },
    }
}

/// Extract a clean, user-friendly tool name from the raw internal name.
///
/// Raw names can include prefixes like `workspace-escape:builtin:write_file`
/// or MCP-style `server:tool_name`. This strips prefixes and returns only the
/// final tool name segment.
fn clean_tool_name(raw: &str) -> String {
    // Strip "workspace-escape:" prefix if present.
    let stripped = raw.strip_prefix("workspace-escape:").unwrap_or(raw);

    // Take only the last colon-separated segment (handles "builtin:write_file",
    // "server:tool_name", etc.).
    stripped.rsplit(':').next().unwrap_or(stripped).to_string()
}

/// Flush the stream buffer into a finalized assistant message.
fn flush_stream_buffer(app: &mut App) {
    if app.stream_buffer.is_empty() {
        return;
    }

    // The stream buffer content has already been appended to the nexus
    // stream and messages in real-time via DaemonEvent::Text handling.
    // Just clear the buffer.
    app.stream_buffer.clear();
}

/// Process pending actions — send approval decisions and input to the daemon.
async fn handle_pending_actions(
    app: &mut App,
    client: &DaemonClient,
    session_id: &SessionId,
) -> anyhow::Result<()> {
    let actions: Vec<PendingAction> = app.pending_actions.drain(..).collect();

    for action in actions {
        match action {
            PendingAction::Approve {
                request_id,
                decision,
            } => {
                let option = match decision {
                    ApprovalDecisionKind::Once => ApprovalOption::AllowOnce,
                    ApprovalDecisionKind::Session => ApprovalOption::AllowSession,
                    ApprovalDecisionKind::Always => ApprovalOption::AllowAlways,
                };
                let request_uuid =
                    uuid::Uuid::parse_str(&request_id).unwrap_or_else(|_| uuid::Uuid::new_v4());
                let approval = ApprovalDecision::new(request_uuid, option);
                if let Err(e) = client
                    .send_approval(session_id, &request_id, approval)
                    .await
                {
                    app.push_notice(&format!("Failed to send approval: {e}"));
                }
            },
            PendingAction::Deny { request_id, reason } => {
                // Check for elicitation cancel sentinel.
                if reason.as_deref() == Some("__elicitation_cancel__") {
                    let request_uuid =
                        uuid::Uuid::parse_str(&request_id).unwrap_or_else(|_| uuid::Uuid::new_v4());
                    let response = ElicitationResponse::cancel(request_uuid);
                    if let Err(e) = client
                        .send_elicitation(session_id, &request_id, response)
                        .await
                    {
                        app.push_notice(&format!("Failed to cancel elicitation: {e}"));
                    }
                } else {
                    let request_uuid =
                        uuid::Uuid::parse_str(&request_id).unwrap_or_else(|_| uuid::Uuid::new_v4());
                    let approval = ApprovalDecision::new(request_uuid, ApprovalOption::Deny);
                    if let Err(e) = client
                        .send_approval(session_id, &request_id, approval)
                        .await
                    {
                        app.push_notice(&format!("Failed to send denial: {e}"));
                    }
                }
            },
            PendingAction::CancelTurn => {
                if let Err(e) = client.cancel_turn(session_id).await {
                    app.push_notice(&format!("Failed to cancel turn: {e}"));
                }
            },
            PendingAction::SendInput(content) => {
                if content.starts_with('/') {
                    handle_slash_command(app, client, session_id, &content).await;
                } else {
                    // Add user message to the stream.
                    app.push_message(MessageRole::User, content.clone());
                    app.scroll_offset = 0;

                    // Start thinking state.
                    app.state = UiState::Thinking {
                        start_time: Instant::now(),
                        dots: 0,
                    };

                    // Send to daemon.
                    if let Err(e) = client.send_input(session_id, &content).await {
                        app.push_notice(&format!("Failed to send input: {e}"));
                        app.state = UiState::Error {
                            message: format!("Send failed: {e}"),
                        };
                    }
                }
            },
        }
    }

    Ok(())
}

/// Handle slash commands, rendering output into the TUI nexus stream.
#[allow(clippy::too_many_lines)]
async fn handle_slash_command(
    app: &mut App,
    client: &DaemonClient,
    session_id: &SessionId,
    command: &str,
) {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let cmd = parts.first().copied().unwrap_or("");
    let arg = parts.get(1).copied();

    match cmd {
        "/help" => {
            app.push_notice(
                "Commands: /help, /clear, /info, /servers, /tools [server], \
                 /plugins, /allowances, /budget, /audit [N], /save, /sessions, exit",
            );
        },
        "/clear" => {
            app.messages.clear();
            app.nexus_stream.clear();
            app.completed_tools.clear();
        },
        "/info" => {
            if let Ok(status) = client.status().await {
                app.push_notice(&format!(
                    "Daemon v{} | Uptime: {}s | Sessions: {} | MCP: {}/{} servers | Plugins: {} loaded",
                    status.version,
                    status.uptime_secs,
                    status.active_sessions,
                    status.mcp_servers_running,
                    status.mcp_servers_configured,
                    status.plugins_loaded,
                ));
            } else {
                app.push_notice("Failed to get daemon info.");
            }
        },
        "/servers" => match client.list_servers().await {
            Ok(servers) if servers.is_empty() => {
                app.push_notice("No MCP servers configured.");
            },
            Ok(servers) => {
                let mut text = String::from("MCP Servers:");
                for s in &servers {
                    let icon = if s.ready { "●" } else { "○" };
                    let _ = write!(text, "\n  {icon} {} ({} tools)", s.name, s.tool_count);
                }
                app.push_notice(&text);
            },
            Err(e) => app.push_notice(&format!("Failed to list servers: {e}")),
        },
        "/tools" => match client.list_tools().await {
            Ok(tools) if tools.is_empty() => {
                app.push_notice("No tools available.");
            },
            Ok(tools) => {
                let filtered: Vec<_> = if let Some(server_filter) = arg {
                    tools.iter().filter(|t| t.server == server_filter).collect()
                } else {
                    tools.iter().collect()
                };

                let mut text = String::from("Available Tools:");
                let mut current_server = "";
                for t in &filtered {
                    if t.server != current_server {
                        current_server = &t.server;
                        let _ = write!(text, "\n  {current_server}");
                    }
                    let desc = t.description.as_deref().unwrap_or("");
                    let _ = write!(text, "\n    {} {desc}", t.name);
                }
                app.push_notice(&text);
            },
            Err(e) => app.push_notice(&format!("Failed to list tools: {e}")),
        },
        "/allowances" => match client.session_allowances(session_id).await {
            Ok(allowances) if allowances.is_empty() => {
                app.push_notice("No active allowances.");
            },
            Ok(allowances) => {
                let mut text = String::from("Active Allowances:");
                for a in &allowances {
                    let scope = if a.session_only {
                        "session"
                    } else {
                        "workspace"
                    };
                    let uses = a
                        .uses_remaining
                        .map_or_else(|| "unlimited".to_string(), |n| format!("{n} uses left"));
                    let _ = write!(text, "\n  [{scope}] {} ({uses})", a.pattern);
                }
                app.push_notice(&text);
            },
            Err(e) => app.push_notice(&format!("Failed to get allowances: {e}")),
        },
        "/budget" => match client.session_budget(session_id).await {
            Ok(budget) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pct = if budget.session_max_usd > 0.0 {
                    (budget.session_spent_usd / budget.session_max_usd * 100.0).clamp(0.0, 100.0)
                        as u8
                } else {
                    0
                };
                let mut text = format!(
                    "Budget: ${:.4} / ${:.2} ({pct}%)",
                    budget.session_spent_usd, budget.session_max_usd,
                );
                let _ = write!(
                    text,
                    "\n  Per-action limit: ${:.2}",
                    budget.per_action_max_usd
                );
                if let Some(ws_spent) = budget.workspace_spent_usd {
                    let ws_max = budget
                        .workspace_max_usd
                        .map_or_else(|| "unlimited".to_string(), |m| format!("${m:.2}"));
                    let _ = write!(text, "\n  Workspace: ${ws_spent:.4} / {ws_max}");
                }
                app.push_notice(&text);
            },
            Err(e) => app.push_notice(&format!("Failed to get budget: {e}")),
        },
        "/audit" => {
            let limit: Option<usize> = arg.and_then(|a| a.parse().ok());
            match client.session_audit(session_id, limit).await {
                Ok(entries) if entries.is_empty() => {
                    app.push_notice("No audit entries.");
                },
                Ok(entries) => {
                    let mut text = String::from("Recent Audit Entries:");
                    for e in &entries {
                        let _ = write!(text, "\n  {} {} {}", e.timestamp, e.action, e.outcome);
                    }
                    app.push_notice(&text);
                },
                Err(e) => app.push_notice(&format!("Failed to get audit log: {e}")),
            }
        },
        "/save" => match client.save_session(session_id).await {
            Ok(()) => app.push_notice("Session saved."),
            Err(e) => app.push_notice(&format!("Failed to save: {e}")),
        },
        "/plugins" => match client.list_plugins().await {
            Ok(plugins) if plugins.is_empty() => {
                app.push_notice("No plugins registered.");
            },
            Ok(plugins) => {
                let mut text = String::from("Plugins:");
                for p in &plugins {
                    let icon = match p.state.as_str() {
                        "ready" => "●",
                        "failed" => "✗",
                        _ => "○",
                    };
                    let error_hint = p
                        .error
                        .as_deref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default();
                    let _ = write!(
                        text,
                        "\n  {icon} {} v{} [{}] ({} tools){error_hint}",
                        p.name, p.version, p.state, p.tool_count,
                    );
                }
                app.push_notice(&text);
            },
            Err(e) => app.push_notice(&format!("Failed to list plugins: {e}")),
        },
        "/sessions" => match client.list_sessions(None).await {
            Ok(sessions) if sessions.is_empty() => {
                app.push_notice("No active sessions.");
            },
            Ok(sessions) => {
                let mut text = String::from("Active Sessions:");
                for s in &sessions {
                    let current = if s.id == *session_id {
                        " (current)"
                    } else {
                        ""
                    };
                    let _ = write!(
                        text,
                        "\n  {} | {} msgs{current}",
                        &s.id.0.to_string()[..8],
                        s.message_count,
                    );
                }
                app.push_notice(&text);
            },
            Err(e) => app.push_notice(&format!("Failed to list sessions: {e}")),
        },
        _ => {
            app.push_notice(&format!("Unknown command: {cmd}. Type /help for commands."));
        },
    }
}
