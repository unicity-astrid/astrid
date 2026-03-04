//! TUI module — ratatui-based terminal interface.
//!
//! Connects the Nexus view to the real daemon via `DaemonClient`.

mod input;
mod render;
pub(crate) mod state;
mod theme;

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use astrid_core::SessionId;
use astrid_events::AstridEvent;
use crossterm::{
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::socket_client::SocketClient;
use state::{App, MessageRole, PendingAction, UiState};

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
    client: &mut SocketClient,
    session_id: &SessionId,
    workspace: Option<std::path::PathBuf>,
    model_name: &str,
) -> anyhow::Result<()> {
    let working_dir = workspace
        .as_ref()
        .map_or_else(|| "no workspace".to_string(), |p| p.display().to_string());
    let session_id_short = session_id.0.to_string()[..8].to_string();

    let mut app = App::new(working_dir, model_name.to_string(), session_id_short);

    // Initialize terminal — wrapped in a guard for proper cleanup.
    let (mut terminal, keyboard_enhanced) = init_terminal()?;
    let result = run_loop(&mut terminal, &mut app, client, session_id).await;

    // Always restore terminal, even on error.
    let _ = restore_terminal(&mut terminal, keyboard_enhanced);

    result
}

/// Inner run loop — separated so terminal restore always happens.
#[allow(clippy::too_many_lines)]
async fn run_loop(
    terminal: &mut Term,
    app: &mut App,
    client: &mut SocketClient,
    session_id: &SessionId,
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

        // Poll for kernel events (non-blocking via timeout).
        match tokio::time::timeout(Duration::from_millis(1), client.read_event()).await {
            Ok(Ok(Some(event))) => {
                handle_daemon_event(app, event);
            },
            Ok(Ok(None)) => {
                // Connection closed.
                app.push_notice("Connection to kernel lost.");
                app.state = UiState::Error {
                    message: "Connection to kernel lost".to_string(),
                };
            },
            Ok(Err(e)) => {
                app.push_notice(&format!("Event error: {e}"));
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
            // Socket connections drop automatically, kernel handles clean disconnect.
            break;
        }
    }

    Ok(())
}

/// Map a `KernelEvent` to TUI state changes.
#[allow(clippy::too_many_lines)]
fn handle_daemon_event(app: &mut App, event: AstridEvent) {
    if let AstridEvent::Ipc { message, .. } = event
        && let astrid_events::ipc::IpcPayload::AgentResponse { text, .. } = message.payload
    {
        app.stream_buffer.push_str(&text);
    }
}

async fn handle_pending_actions(
    app: &mut App,
    client: &mut SocketClient,
    session_id: &SessionId,
) -> anyhow::Result<()> {
    let actions: Vec<PendingAction> = app.pending_actions.drain(..).collect();

    for action in actions {
        match action {
            PendingAction::Approve { .. } => {
                // TODO: Translate to KernelRequest::ApproveCapability
                app.push_notice("Approval via UI is temporarily disabled in Microkernel mode.");
            },
            PendingAction::Deny { .. } => {
                // TODO: Translate to KernelRequest::ApproveCapability (deny)
                app.push_notice("Denial via UI is temporarily disabled in Microkernel mode.");
            },
            PendingAction::CancelTurn => {
                // Send an empty UserInput with a special __cancel__ context
                // This signals to the Orchestrator to abort the current ReAct loop
                let cancel_payload = astrid_events::ipc::IpcPayload::UserInput {
                    text: String::new(),
                    context: Some(serde_json::json!({"action": "cancel_turn"})),
                };
                let msg = astrid_events::ipc::IpcMessage::new(
                    "user.prompt",
                    cancel_payload,
                    session_id.0,
                );
                if let Err(e) = client.send_message(msg).await {
                    app.push_notice(&format!("Failed to send cancellation signal: {e}"));
                } else {
                    app.state = UiState::Interrupted;
                }
            },
            PendingAction::SendInput(content) => {
                if content.starts_with('/') {
                    handle_slash_command(&content, app, client, session_id);
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
                    if let Err(e) = client.send_input(content).await {
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
fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    _client: &mut SocketClient,
    _session_id: &SessionId,
) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }

    match parts[0] {
        "/quit" | "/exit" | "/q" => {
            app.should_quit = true;
        }
        "/clear" => {
            app.messages.clear();
            app.nexus_stream.clear();
            app.stream_buffer.clear();
            app.push_notice("Screen cleared.");
        }
        "/help" => {
            app.push_message(MessageRole::User, cmd.to_string());
            app.push_message(MessageRole::Assistant, 
                "**Available UI Commands:**\n\
                 - `/help`   - Show this message\n\
                 - `/clear`  - Clear the local terminal screen\n\
                 - `/quit`   - Disconnect from the OS Kernel\n\
                 \n\
                 *Note: To manage sessions, use the native `astrid session` command outside the TUI.*".to_string()
            );
        }
        _ => {
            app.push_notice(&format!("Unknown UI command: {cmd}. Type /help for available commands."));
        }
    }
}
