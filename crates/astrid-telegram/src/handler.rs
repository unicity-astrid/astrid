//! Message handler: receives text from Telegram, sends to daemon, starts
//! event loop.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::{info, warn};

use crate::approval::ApprovalManager;
use crate::client::DaemonClient;
use crate::config::TelegramConfig;
use crate::elicitation::ElicitationManager;
use crate::event_loop::run_event_loop;
use crate::session::{SessionMap, TurnStartResult};

/// Shared bot state passed to all handlers.
#[derive(Clone)]
pub struct BotState {
    pub daemon: Arc<DaemonClient>,
    pub sessions: SessionMap,
    pub config: Arc<TelegramConfig>,
    pub approvals: ApprovalManager,
    pub elicitations: ElicitationManager,
}

/// Handle an incoming text message.
pub async fn handle_message(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    let Some(text) = msg.text() else {
        return Ok(());
    };

    let chat_id = msg.chat.id;

    // Access control: check user identity against the allowlist.
    // If msg.from is absent (channel posts, etc.) and an allowlist is set,
    // deny access since we can't verify the sender.
    let user_allowed = match &msg.from {
        Some(user) => state.config.is_user_allowed(user.id.0),
        None => state.config.allowed_user_ids.is_empty(),
    };
    if !user_allowed {
        let _ = bot
            .send_message(chat_id, "You are not authorized to use this bot.")
            .await;
        return Ok(());
    }

    // Check if this is an elicitation text reply.
    if state
        .elicitations
        .handle_text_reply(chat_id, text, &state.daemon, &state.sessions)
        .await
    {
        return Ok(());
    }

    // Handle bot commands.
    if text.starts_with('/') {
        return handle_command(&bot, chat_id, text, &state).await;
    }

    // Ensure session exists and atomically start the turn.
    let session_id = match acquire_session_and_start_turn(&state, chat_id).await {
        Ok(sid) => sid,
        Err(msg) => {
            let _ = bot.send_message(chat_id, msg).await;
            return Ok(());
        },
    };

    // Send "Thinking..." placeholder.
    let placeholder = match bot.send_message(chat_id, "Thinking...").await {
        Ok(msg) => msg,
        Err(e) => {
            warn!("Failed to send placeholder: {e}");
            state.sessions.set_turn_in_progress(chat_id, false).await;
            return Ok(());
        },
    };

    // Send typing indicator.
    let _ = bot
        .send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
        .await;

    // Send input to daemon.
    if let Err(e) = state.daemon.send_input(&session_id, text).await {
        let _ = bot
            .edit_message_text(chat_id, placeholder.id, format!("Error: {e}"))
            .await;
        state.sessions.set_turn_in_progress(chat_id, false).await;
        return Ok(());
    }

    // Subscribe to events.
    let subscription = match state.daemon.subscribe_events(&session_id).await {
        Ok(sub) => sub,
        Err(e) => {
            let _ = bot
                .edit_message_text(chat_id, placeholder.id, format!("Failed to subscribe: {e}"))
                .await;
            state.sessions.set_turn_in_progress(chat_id, false).await;
            return Ok(());
        },
    };

    // Spawn event loop task.
    tokio::spawn(run_event_loop(
        bot.clone(),
        chat_id,
        placeholder.id,
        subscription,
        state.sessions.clone(),
        state.approvals.clone(),
        state.elicitations.clone(),
    ));

    Ok(())
}

/// Ensure a session exists for `chat_id` and atomically start a turn.
///
/// Returns `Ok(session_id)` on success, or `Err(user_message)` with a message
/// to display to the user.
async fn acquire_session_and_start_turn(
    state: &BotState,
    chat_id: ChatId,
) -> Result<astrid_core::SessionId, &'static str> {
    match state.sessions.try_start_existing_turn(chat_id).await {
        TurnStartResult::Started(sid) => return Ok(sid),
        TurnStartResult::TurnBusy => {
            return Err("A turn is already in progress. Please wait.");
        },
        TurnStartResult::NoSession => {},
    }

    // No session — claim creation lock to prevent duplicate create_session
    // calls when concurrent messages race for the same chat.
    if !state.sessions.try_claim_creation(chat_id).await {
        return Err("Session is being created. Please wait.");
    }

    let workspace = state.config.workspace_path.as_ref().map(PathBuf::from);
    match state.daemon.create_session(workspace).await {
        Ok(session_info) => {
            info!("Created session {} for chat {}", session_info.id, chat_id);
            // Atomically insert session + start turn under one lock to
            // prevent another message from stealing the turn in between.
            let sid = state
                .sessions
                .finish_creation_and_start_turn(chat_id, session_info.id)
                .await;
            Ok(sid)
        },
        Err(e) => {
            warn!("Failed to create session for chat {chat_id}: {e}");
            state.sessions.cancel_creation(chat_id).await;
            Err("Failed to create session. Please try again.")
        },
    }
}

/// Handle bot commands.
async fn handle_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &BotState,
) -> anyhow::Result<()> {
    let cmd = text.split_whitespace().next().unwrap_or("");

    match cmd {
        "/start" => {
            let msg = "Welcome to Astrid! Send me a message and I'll process it \
                       through the agent runtime.\n\n\
                       Commands:\n\
                       /help - Show this help\n\
                       /reset - Reset session\n\
                       /status - Daemon status\n\
                       /cancel - Cancel current turn";
            let _ = bot.send_message(chat_id, msg).await;
        },
        "/help" => {
            let msg = "<b>Astrid Telegram Bot</b>\n\n\
                       Send any text message to interact with the agent.\n\n\
                       <b>Commands:</b>\n\
                       /start - Welcome message\n\
                       /help - This help text\n\
                       /reset - End current session and start fresh\n\
                       /status - Show daemon status and budget\n\
                       /cancel - Cancel the current turn";
            let _ = bot
                .send_message(chat_id, msg)
                .parse_mode(ParseMode::Html)
                .await;
        },
        "/reset" => {
            if let Some(session_id) = state.sessions.remove(chat_id).await {
                let _ = state.daemon.end_session(&session_id).await;
            }
            let _ = bot.send_message(chat_id, "Session reset.").await;
        },
        "/status" => match state.daemon.status().await {
            Ok(status) => {
                let mut msg = format!(
                    "<b>Daemon Status</b>\n\
                         Uptime: {}s\n\
                         Active sessions: {}\n\
                         Version: {}",
                    status.uptime_secs,
                    status.active_sessions,
                    crate::format::html_escape(&status.version),
                );

                if let Some(session_id) = state.sessions.get_session_id(chat_id).await
                    && let Ok(budget) = state.daemon.session_budget(&session_id).await
                {
                    let _ = write!(
                        msg,
                        "\n\n<b>Budget</b>\n\
                             Spent: ${:.4}\n\
                             Remaining: ${:.4}\n\
                             Limit: ${:.4}",
                        budget.session_spent_usd,
                        budget.session_remaining_usd,
                        budget.session_max_usd,
                    );
                }

                let _ = bot
                    .send_message(chat_id, msg)
                    .parse_mode(ParseMode::Html)
                    .await;
            },
            Err(e) => {
                let _ = bot
                    .send_message(chat_id, format!("Failed to get status: {e}"))
                    .await;
            },
        },
        "/cancel" => {
            if let Some(session_id) = state.sessions.get_session_id(chat_id).await {
                match state.daemon.cancel_turn(&session_id).await {
                    Ok(()) => {
                        state.sessions.set_turn_in_progress(chat_id, false).await;
                        let _ = bot.send_message(chat_id, "Turn cancelled.").await;
                    },
                    Err(e) => {
                        let _ = bot
                            .send_message(chat_id, format!("Failed to cancel: {e}"))
                            .await;
                    },
                }
            } else {
                let _ = bot.send_message(chat_id, "No active session.").await;
            }
        },
        _ => {
            let _ = bot
                .send_message(chat_id, "Unknown command. Try /help.")
                .await;
        },
    }

    Ok(())
}
