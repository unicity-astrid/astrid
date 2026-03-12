//! TUI module — ratatui-based terminal interface.
//!
//! Connects the Nexus view to the real daemon via `DaemonClient`.

mod input;
mod render;
pub(crate) mod state;
mod theme;

use std::io::{self, Stdout, Write as _};
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

    // Sync dynamic commands on startup.
    let req = astrid_events::kernel_api::KernelRequest::GetCommands;
    if let Ok(val) = serde_json::to_value(req) {
        let msg = astrid_events::ipc::IpcMessage::new(
            "astrid.v1.request.get_commands",
            astrid_events::ipc::IpcPayload::RawJson(val),
            session_id.0,
        );
        let _ = client.send_message(msg).await;
    }

    let result = run_loop(&mut terminal, &mut app, client, session_id).await;

    // Always restore terminal, even on error.
    let _ = restore_terminal(&mut terminal, keyboard_enhanced);

    result
}

/// Inner run loop — separated so terminal restore always happens.
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
            app.terminal_height = terminal.size()?.height;
            terminal.draw(|frame| render::render_frame(frame, app))?;
            last_render = Instant::now();
        }

        // Process pending actions (approval decisions, input sends).
        handle_pending_actions(app, client, session_id, terminal).await?;

        // Poll for crossterm input events (non-blocking).
        if crossterm::event::poll(Duration::from_millis(10))? {
            input::handle_input(app)?;
        }

        // Clear transient status messages after 5 seconds
        if let Some((_, time)) = &app.status_message
            && time.elapsed() > Duration::from_secs(5)
        {
            app.status_message = None;
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
                    message: "Connection to kernel lost. Press Q to quit.".to_string(),
                };
                // Don't break immediately, let the user read the error and quit.
                // But we must prevent an infinite loop of pushing notices.
                // Wait, if we don't break, `read_event` will instantly return `Ok(None)` again and again.
                // Let's just set the state and break, or use a flag.
                break;
            },
            Ok(Err(e)) => {
                app.push_notice(&format!("Event error: {e}"));
                break;
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
            // Notify the kernel so it can update its connection count.
            // NOTE: This message travels over the socket to the WASM proxy
            // capsule, which must re-publish it on the EventBus as
            // `client.v1.disconnect` for the ConnectionTracker to see it.
            // If the proxy doesn't forward it, the secondary signal
            // (bus subscriber_count drop) still handles idle detection.
            let msg = astrid_events::ipc::IpcMessage::new(
                "client.v1.disconnect",
                astrid_events::ipc::IpcPayload::Disconnect {
                    reason: Some("quit".to_string()),
                },
                session_id.0,
            );
            let _ = client.send_message(msg).await;
            break;
        }
    }

    Ok(())
}

/// Map a `KernelEvent` to TUI state changes.
#[expect(clippy::too_many_lines)]
fn handle_daemon_event(app: &mut App, event: AstridEvent) {
    if let AstridEvent::Ipc { message, .. } = event {
        if let astrid_events::ipc::IpcPayload::AgentResponse { text, is_final, .. } =
            &message.payload
        {
            // Transition to streaming state on first non-empty delta
            if !text.is_empty() && !matches!(app.state, UiState::Streaming { .. }) {
                app.state = UiState::Streaming {
                    start_time: Instant::now(),
                };
            }
            app.stream_buffer.push_str(text);

            if *is_final {
                // Flush the accumulated stream buffer as an assistant message
                if !app.stream_buffer.is_empty() {
                    let response = std::mem::take(&mut app.stream_buffer);
                    app.push_message(MessageRole::Assistant, response);
                }
                app.state = UiState::Idle;
                app.scroll_offset = 0;
            }
        } else if let astrid_events::ipc::IpcPayload::OnboardingRequired { capsule_id, fields } =
            &message.payload
        {
            if fields.is_empty() {
                app.push_notice(&format!(
                    "Capsule '{capsule_id}' reported missing configuration but provided no fields."
                ));
                return;
            }

            let msg = format!("Action required: Capsule '{capsule_id}' requires configuration.");
            app.push_notice(&msg);
            app.status_message = Some((msg, Instant::now()));

            let first = fields.first();
            let is_first_enum = first.is_some_and(|f| {
                matches!(
                    f.field_type,
                    astrid_events::ipc::OnboardingFieldType::Enum(_)
                )
            });
            let enum_selected = first.map_or(0, input::default_enum_position);
            let default_val = first.and_then(|f| f.default.clone()).unwrap_or_default();

            app.state = UiState::Onboarding {
                capsule_id: capsule_id.clone(),
                fields: fields.clone(),
                current_idx: 0,
                answers: std::collections::HashMap::new(),
                enum_selected,
                enum_scroll_offset: 0,
                current_array_items: Vec::new(),
            };
            let is_first_array = first.is_some_and(|f| {
                matches!(f.field_type, astrid_events::ipc::OnboardingFieldType::Array)
            });
            input::prefill_field_input(app, is_first_enum || is_first_array, &default_val);
        } else if let astrid_events::ipc::IpcPayload::ElicitRequest {
            request_id,
            capsule_id,
            field,
        } = &message.payload
        {
            let msg = format!("Capsule '{capsule_id}' is requesting input: {}", field.key);
            app.push_notice(&msg);
            app.status_message = Some((msg, Instant::now()));

            // Store the elicit request ID so the input handler knows to
            // publish an ElicitResponse instead of writing .env.json.
            app.elicit_request_id = Some(*request_id);

            let is_enum = matches!(
                field.field_type,
                astrid_events::ipc::OnboardingFieldType::Enum(_)
            );
            let is_array = matches!(
                field.field_type,
                astrid_events::ipc::OnboardingFieldType::Array
            );
            let enum_selected = input::default_enum_position(field);
            let default_val = field.default.clone().unwrap_or_default();

            app.state = UiState::Onboarding {
                capsule_id: capsule_id.clone(),
                fields: vec![field.clone()],
                current_idx: 0,
                answers: std::collections::HashMap::new(),
                enum_selected,
                enum_scroll_offset: 0,
                current_array_items: Vec::new(),
            };
            input::prefill_field_input(app, is_enum || is_array, &default_val);
        } else if let astrid_events::ipc::IpcPayload::SelectionRequired {
            request_id,
            title,
            options,
            callback_topic,
        } = &message.payload
        {
            if options.is_empty() {
                app.push_notice("No options available.");
            } else {
                app.state = UiState::Selection {
                    title: title.clone(),
                    options: options.clone(),
                    selected: 0,
                    scroll_offset: 0,
                    callback_topic: callback_topic.clone(),
                    request_id: request_id.clone(),
                };
            }
        } else if let astrid_events::ipc::IpcPayload::RawJson(val) = &message.payload
            && let Ok(astrid_events::kernel_api::KernelResponse::Commands(cmds)) =
                serde_json::from_value::<astrid_events::kernel_api::KernelResponse>(val.clone())
        {
            // Reset the dynamic slash command palette to the hardcoded base commands
            app.slash_commands = vec![
                state::SlashCommandDef {
                    name: "/help".to_string(),
                    description: "Show available commands".to_string(),
                },
                state::SlashCommandDef {
                    name: "/clear".to_string(),
                    description: "Clear conversation history".to_string(),
                },
                state::SlashCommandDef {
                    name: "/install".to_string(),
                    description: "Install a capsule from a path or registry".to_string(),
                },
                state::SlashCommandDef {
                    name: "/refresh".to_string(),
                    description: "Reload all installed capsules into the OS".to_string(),
                },
                state::SlashCommandDef {
                    name: "/quit".to_string(),
                    description: "Disconnect from the daemon".to_string(),
                },
            ];

            // Append all dynamically discovered capsule commands
            for cmd in &cmds {
                app.slash_commands.push(state::SlashCommandDef {
                    name: format!("/{}", cmd.name),
                    description: format!("{} (via {})", cmd.description, cmd.provider_capsule),
                });
            }
            tracing::debug!(
                dynamic_commands = cmds.len(),
                total = app.slash_commands.len(),
                "Refreshed slash command palette"
            );
        }

        // When the kernel finishes loading all capsules, re-fetch commands
        // so dynamic slash commands (like /models) appear even if the CLI
        // connected before non-uplink capsules were loaded.
        if message.topic == "astrid.v1.capsules_loaded" {
            app.pending_actions
                .push(state::PendingAction::RefreshCommands);
        }

        // Registry responses
        if message.topic == "registry.v1.response.get_providers" {
            if let astrid_events::ipc::IpcPayload::Custom { data } = &message.payload
                && let Some(providers) = data.as_array()
            {
                if providers.is_empty() {
                    app.push_notice("No LLM providers are currently loaded.");
                } else {
                    use std::fmt::Write as _;
                    let mut text = String::from("Available Models:\n");
                    for p in providers {
                        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let desc = p.get("description").and_then(|v| v.as_str()).unwrap_or("");
                        let capsule = p.get("capsule").and_then(|v| v.as_str()).unwrap_or("?");
                        let _ = writeln!(text, "  - {id} — {desc} (via {capsule})");
                    }
                    text.push_str("\nUse /models <model_id> to switch.");
                    app.push_message(MessageRole::LocalUi, text);
                }
            }
        } else if message.topic == "registry.v1.response.set_active_model" {
            if let astrid_events::ipc::IpcPayload::Custom { data } = &message.payload {
                if let Some(model) = data
                    .get("active_model")
                    .and_then(|m| m.get("id"))
                    .and_then(|v| v.as_str())
                {
                    app.push_notice(&format!("Active model set to: {model}"));
                    app.model_name = model.to_string();
                } else if let Some(err) = data.get("error").and_then(|v| v.as_str()) {
                    app.push_notice(&format!("Failed to set model: {err}"));
                }
            }
        } else if message.topic == "registry.v1.active_model_changed"
            && let astrid_events::ipc::IpcPayload::Custom { data } = &message.payload
            && let Some(id) = data.get("id").and_then(|v| v.as_str())
        {
            app.model_name = id.to_string();
        }
    }
}

#[expect(clippy::too_many_lines)]
async fn handle_pending_actions(
    app: &mut App,
    client: &mut SocketClient,
    session_id: &SessionId,
    terminal: &mut Term,
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
                // This signals to the react capsule to abort the current loop
                let cancel_payload = astrid_events::ipc::IpcPayload::UserInput {
                    text: String::new(),
                    session_id: session_id.0.to_string(),
                    context: Some(serde_json::json!({"action": "cancel_turn"})),
                };
                let msg = astrid_events::ipc::IpcMessage::new(
                    "user.v1.prompt",
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
                    handle_slash_command(&content, app, client, session_id, terminal).await;
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
            PendingAction::SubmitSelection {
                callback_topic,
                request_id,
                selected_id,
                selected_label,
            } => {
                app.push_notice(&format!("Selected: {selected_label}"));
                let msg = astrid_events::ipc::IpcMessage::new(
                    callback_topic,
                    astrid_events::ipc::IpcPayload::Custom {
                        data: serde_json::json!({
                            "request_id": request_id,
                            "selected_id": selected_id,
                        }),
                    },
                    session_id.0,
                );
                let _ = client.send_message(msg).await;
            },
            PendingAction::RefreshCommands => {
                let req = astrid_events::kernel_api::KernelRequest::GetCommands;
                if let Ok(val) = serde_json::to_value(req) {
                    let msg = astrid_events::ipc::IpcMessage::new(
                        "astrid.v1.request.get_commands",
                        astrid_events::ipc::IpcPayload::RawJson(val),
                        session_id.0,
                    );
                    let _ = client.send_message(msg).await;
                }
            },
            PendingAction::SubmitOnboarding {
                capsule_id,
                answers,
            } => {
                if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
                    let env_path = home.capsules_dir().join(&capsule_id).join(".env.json");
                    if let Ok(json) = serde_json::to_string_pretty(&answers) {
                        if let Err(e) = write_env_file(&env_path, &json) {
                            app.push_notice(&format!("Failed to save configuration: {e}"));
                        } else {
                            let msg = "Configuration saved. Refreshing Kernel...";
                            app.push_notice(msg);
                            app.status_message = Some((msg.to_string(), Instant::now()));

                            let req = astrid_events::kernel_api::KernelRequest::ReloadCapsules;
                            if let Ok(val) = serde_json::to_value(req) {
                                let ipc_msg = astrid_events::ipc::IpcMessage::new(
                                    "astrid.v1.request.reload_capsules",
                                    astrid_events::ipc::IpcPayload::RawJson(val),
                                    session_id.0,
                                );
                                let _ = client.send_message(ipc_msg).await;
                            }
                        }
                    }
                }
            },
            PendingAction::SubmitElicitResponse {
                request_id,
                value,
                values,
            } => {
                let response_topic = format!("astrid.v1.elicit.response.{request_id}");
                let response = astrid_events::ipc::IpcPayload::ElicitResponse {
                    request_id,
                    value,
                    values,
                };
                let msg =
                    astrid_events::ipc::IpcMessage::new(response_topic, response, session_id.0);
                let _ = client.send_message(msg).await;
                app.push_notice("Lifecycle input submitted.");
            },
        }
    }

    Ok(())
}

/// Handle slash commands, rendering output into the TUI nexus stream.
#[expect(clippy::too_many_lines)]
async fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    client: &mut SocketClient,
    session_id: &SessionId,
    terminal: &mut Term,
) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }

    match parts[0] {
        "/quit" | "/exit" | "/q" => {
            app.should_quit = true;
        },
        "/clear" => {
            app.messages.clear();
            app.nexus_stream.clear();
            app.stream_buffer.clear();
        },
        "/install" => {
            app.push_message(MessageRole::User, cmd.to_string());
            if parts.len() < 2 {
                app.push_notice("Usage: /install <path-to-capsule-or-directory>");
            } else {
                let source = parts[1];
                let msg = format!("Installing capsule from: {source}...");
                app.push_notice(&msg);
                app.status_message = Some((msg, Instant::now()));

                // Force a redraw before starting blocking task
                let _ = terminal.draw(|frame| render::render_frame(frame, app));

                let source_owned = source.to_string();
                let result = tokio::task::spawn_blocking(move || {
                    crate::commands::capsule::install::install_capsule(&source_owned, false)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("Task panicked: {e}")));

                match result {
                    Ok(()) => {
                        let success_msg =
                            "Installation complete. Sending refresh signal to Kernel...";
                        app.push_notice(success_msg);
                        app.status_message = Some((success_msg.to_string(), Instant::now()));

                        let req = astrid_events::kernel_api::KernelRequest::ReloadCapsules;
                        if let Ok(val) = serde_json::to_value(req) {
                            let msg = astrid_events::ipc::IpcMessage::new(
                                "astrid.v1.request.reload_capsules",
                                astrid_events::ipc::IpcPayload::RawJson(val),
                                session_id.0,
                            );
                            let _ = client.send_message(msg).await;
                        }

                        // Refresh the slash command palette so newly installed
                        // capsule commands appear without restarting the CLI.
                        let req = astrid_events::kernel_api::KernelRequest::GetCommands;
                        if let Ok(val) = serde_json::to_value(req) {
                            let msg = astrid_events::ipc::IpcMessage::new(
                                "astrid.v1.request.get_commands",
                                astrid_events::ipc::IpcPayload::RawJson(val),
                                session_id.0,
                            );
                            let _ = client.send_message(msg).await;
                        }
                    },
                    Err(e) => {
                        app.push_notice(&format!("Failed to install capsule: {e}"));
                    },
                }
            }
        },
        "/refresh" => {
            app.push_message(MessageRole::User, cmd.to_string());
            app.push_notice("Sending refresh signal to daemon...");
            let req = astrid_events::kernel_api::KernelRequest::ReloadCapsules;
            if let Ok(val) = serde_json::to_value(req) {
                let msg = astrid_events::ipc::IpcMessage::new(
                    "astrid.v1.request.reload_capsules",
                    astrid_events::ipc::IpcPayload::RawJson(val),
                    session_id.0,
                );
                let _ = client.send_message(msg).await;
            }

            // Refresh the slash command palette after reload.
            let req = astrid_events::kernel_api::KernelRequest::GetCommands;
            if let Ok(val) = serde_json::to_value(req) {
                let msg = astrid_events::ipc::IpcMessage::new(
                    "astrid.v1.request.get_commands",
                    astrid_events::ipc::IpcPayload::RawJson(val),
                    session_id.0,
                );
                let _ = client.send_message(msg).await;
            }
        },
        "/help" | "?" => {
            app.push_message(MessageRole::User, cmd.to_string());
            app.push_message(
                MessageRole::LocalUi,
                "**Available UI Commands:**\n\
                 - `/help`     - Show this message\n\
                 - `/clear`    - Clear the local terminal screen\n\
                 - `/install`  - Install and load a capsule\n\
                 - `/refresh`  - Reload all capsules into the OS\n\
                 - `/quit`     - Disconnect from the daemon\n\
                 \n\
                 Capsule commands (from installed capsules) also appear in the palette."
                    .to_string(),
            );
            let req = astrid_events::kernel_api::KernelRequest::GetCommands;
            if let Ok(val) = serde_json::to_value(req) {
                let msg = astrid_events::ipc::IpcMessage::new(
                    "astrid.v1.request.get_commands",
                    astrid_events::ipc::IpcPayload::RawJson(val),
                    session_id.0,
                );
                let _ = client.send_message(msg).await;
            }
        },
        _ => {
            // It's a custom command! Route it to the Event Bus for capsules to handle.
            let msg = astrid_events::ipc::IpcMessage::new(
                "cli.v1.command.execute",
                astrid_events::ipc::IpcPayload::UserInput {
                    text: cmd.to_string(),
                    session_id: session_id.0.to_string(),
                    context: None,
                },
                session_id.0,
            );

            if let Err(e) = client.send_message(msg).await {
                app.push_notice(&format!("Failed to send command to Kernel: {e}"));
            } else {
                app.push_message(MessageRole::User, cmd.to_string());
                app.state = UiState::Thinking {
                    start_time: Instant::now(),
                    dots: 0,
                };
            }
        },
    }
}

/// Write `.env.json` with restricted permissions (0o600 on Unix).
fn write_env_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    // Ensure parent directory exists (capsule dir may not have been written to yet).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        file.flush()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}
