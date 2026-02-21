//! Approval flow via Telegram inline keyboards.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use astrid_core::{ApprovalDecision, ApprovalOption, ApprovalRequest};
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::RwLock;
use tracing::warn;

use crate::client::DaemonClient;
use crate::format::html_escape;
use crate::session::SessionMap;

/// Pending approvals older than this are automatically reaped.
const PENDING_TTL: Duration = Duration::from_secs(5 * 60);

/// A pending approval waiting for the user to press a button.
struct PendingApproval {
    /// Full UUID string of the approval request.
    request_id: String,
    /// The chat this approval belongs to.
    chat_id: ChatId,
    /// The available options.
    options: Vec<ApprovalOption>,
    /// When this entry was created (for TTL expiry).
    created_at: Instant,
}

/// Manages pending approval requests.
#[derive(Clone)]
pub struct ApprovalManager {
    /// Map from full `request_id` to pending approval.
    pending: Arc<RwLock<HashMap<String, PendingApproval>>>,
}

impl Default for ApprovalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalManager {
    /// Create a new, empty approval manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Send an approval request as a Telegram message with inline keyboard.
    pub async fn send_approval(
        &self,
        bot: &Bot,
        chat_id: ChatId,
        request_id: &str,
        request: &ApprovalRequest,
    ) {
        let risk_label = format!("{:?}", request.risk_level);
        let mut text = format!(
            "<b>Approval Required</b> [{}]\n\n<b>{}</b>\n{}",
            html_escape(&risk_label),
            html_escape(&request.operation),
            html_escape(&request.description),
        );
        if let Some(ref resource) = request.resource {
            let _ = write!(text, "\n\nResource: <code>{}</code>", html_escape(resource),);
        }

        // Use full request_id in callback data (UUIDs are 36 chars, well
        // within Telegram's 64-byte callback_data limit after the "apr:" prefix
        // and option index).
        let buttons: Vec<InlineKeyboardButton> = request
            .options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                let label = opt.to_string();
                let callback = format!("apr:{request_id}:{i}");
                InlineKeyboardButton::callback(label, callback)
            })
            .collect();

        let keyboard: Vec<Vec<InlineKeyboardButton>> = buttons
            .chunks(2)
            .map(<[InlineKeyboardButton]>::to_vec)
            .collect();

        let markup = InlineKeyboardMarkup::new(keyboard);

        if let Err(e) = bot
            .send_message(chat_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(markup)
            .await
        {
            warn!("Failed to send approval message: {e}");
            return;
        }

        let mut guard = self.pending.write().await;
        // Reap expired entries before inserting to bound memory usage.
        guard.retain(|_, v| v.created_at.elapsed() < PENDING_TTL);
        guard.insert(
            request_id.to_string(),
            PendingApproval {
                request_id: request_id.to_string(),
                chat_id,
                options: request.options.clone(),
                created_at: Instant::now(),
            },
        );
    }

    /// Handle a callback query from an approval button press.
    ///
    /// Callback data format: `apr:{request_id}:{option_index}`
    pub async fn handle_callback(
        &self,
        bot: &Bot,
        query: &CallbackQuery,
        daemon: &DaemonClient,
        sessions: &SessionMap,
    ) -> bool {
        let data = match query.data.as_ref() {
            Some(d) if d.starts_with("apr:") => d,
            _ => return false,
        };

        let parts: Vec<&str> = data.splitn(3, ':').collect();
        if parts.len() != 3 {
            return false;
        }

        let prefix = parts[1];
        let Ok(option_idx) = parts[2].parse::<usize>() else {
            return false;
        };

        let pending = self.pending.write().await.remove(prefix);
        let Some(pending) = pending else {
            let _ = bot.answer_callback_query(&query.id).text("Expired").await;
            return true;
        };

        let Some(option) = pending.options.get(option_idx).copied() else {
            let _ = bot
                .answer_callback_query(&query.id)
                .text("Invalid option")
                .await;
            return true;
        };

        let Some(session_id) = sessions.get_session_id(pending.chat_id).await else {
            let _ = bot
                .answer_callback_query(&query.id)
                .text("No active session")
                .await;
            return true;
        };

        let Ok(request_id_uuid) = uuid::Uuid::parse_str(&pending.request_id) else {
            let _ = bot
                .answer_callback_query(&query.id)
                .text("Invalid request")
                .await;
            return true;
        };

        let decision = ApprovalDecision::new(request_id_uuid, option);

        if let Err(e) = daemon
            .send_approval(&session_id, &pending.request_id, decision)
            .await
        {
            warn!("Failed to send approval response: {e}");
            let _ = bot
                .answer_callback_query(&query.id)
                .text("Error sending response")
                .await;
            return true;
        }

        let label = option.to_string();
        let _ = bot
            .answer_callback_query(&query.id)
            .text(format!("Selected: {label}"))
            .await;

        if let Some(msg) = &query.message {
            let msg_id = msg.id();
            let _ = bot
                .edit_message_reply_markup(pending.chat_id, msg_id)
                .reply_markup(InlineKeyboardMarkup::new(
                    Vec::<Vec<InlineKeyboardButton>>::new(),
                ))
                .await;
            let _ = bot
                .send_message(pending.chat_id, format!("Decision: <b>{label}</b>"))
                .parse_mode(ParseMode::Html)
                .await;
        }

        true
    }
}
