//! Elicitation flow via Telegram inline keyboards and text replies.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use astrid_core::{ElicitationRequest, ElicitationResponse, ElicitationSchema};
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::RwLock;
use tracing::warn;

use crate::client::DaemonClient;
use crate::format::html_escape;
use crate::session::SessionMap;

/// Pending elicitations older than this are automatically reaped.
const PENDING_TTL: Duration = Duration::from_secs(5 * 60);

/// A pending elicitation waiting for a text reply from the user.
struct PendingTextReply {
    request_id: String,
    created_at: Instant,
}

/// A pending elicitation waiting for a keyboard button press.
struct PendingCallback {
    request_id: String,
    chat_id: ChatId,
    values: Vec<String>,
    created_at: Instant,
}

/// Manages elicitation requests.
#[derive(Clone)]
pub struct ElicitationManager {
    /// Pending keyboard-based elicitations: full `request_id` to pending.
    pending_callbacks: Arc<RwLock<HashMap<String, PendingCallback>>>,
    /// Pending text-reply elicitations: `ChatId` to pending.
    pending_text_replies: Arc<RwLock<HashMap<ChatId, PendingTextReply>>>,
}

impl Default for ElicitationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ElicitationManager {
    pub fn new() -> Self {
        Self {
            pending_callbacks: Arc::new(RwLock::new(HashMap::new())),
            pending_text_replies: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Send an elicitation request to the user.
    pub async fn send_elicitation(
        &self,
        bot: &Bot,
        chat_id: ChatId,
        request_id: &str,
        request: &ElicitationRequest,
    ) {
        let header = format!(
            "<b>{}</b>\n{}",
            html_escape(&request.server_name),
            html_escape(&request.message),
        );

        match &request.schema {
            ElicitationSchema::Select { options, .. } => {
                self.send_select(bot, chat_id, request_id, &header, options)
                    .await;
            },
            ElicitationSchema::Confirm { default } => {
                self.send_confirm(bot, chat_id, request_id, &header, *default)
                    .await;
            },
            ElicitationSchema::Text { placeholder, .. }
            | ElicitationSchema::Secret { placeholder } => {
                self.send_text_prompt(bot, chat_id, request_id, &header, placeholder.as_deref())
                    .await;
            },
        }
    }

    /// Send a select-style elicitation with one button per option.
    async fn send_select(
        &self,
        bot: &Bot,
        chat_id: ChatId,
        request_id: &str,
        header: &str,
        options: &[astrid_core::SelectOption],
    ) {
        let values: Vec<String> = options.iter().map(|o| o.value.clone()).collect();

        let mut buttons: Vec<Vec<InlineKeyboardButton>> = options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                let callback = format!("eli:{request_id}:{i}");
                vec![InlineKeyboardButton::callback(&opt.label, callback)]
            })
            .collect();

        buttons.push(vec![InlineKeyboardButton::callback(
            "Cancel",
            format!("eli:{request_id}:cancel"),
        )]);

        let markup = InlineKeyboardMarkup::new(buttons);

        if let Err(e) = bot
            .send_message(chat_id, header)
            .parse_mode(ParseMode::Html)
            .reply_markup(markup)
            .await
        {
            warn!("Failed to send elicitation: {e}");
            return;
        }

        let mut guard = self.pending_callbacks.write().await;
        guard.retain(|_, v| v.created_at.elapsed() < PENDING_TTL);
        guard.insert(
            request_id.to_string(),
            PendingCallback {
                request_id: request_id.to_string(),
                chat_id,
                values,
                created_at: Instant::now(),
            },
        );
    }

    /// Send a yes/no confirmation elicitation.
    async fn send_confirm(
        &self,
        bot: &Bot,
        chat_id: ChatId,
        request_id: &str,
        header: &str,
        default: bool,
    ) {
        let yes_label = if default { "Yes (default)" } else { "Yes" };
        let no_label = if default { "No" } else { "No (default)" };

        let buttons = vec![vec![
            InlineKeyboardButton::callback(yes_label, format!("eli:{request_id}:yes")),
            InlineKeyboardButton::callback(no_label, format!("eli:{request_id}:no")),
        ]];
        let markup = InlineKeyboardMarkup::new(buttons);

        if let Err(e) = bot
            .send_message(chat_id, header)
            .parse_mode(ParseMode::Html)
            .reply_markup(markup)
            .await
        {
            warn!("Failed to send elicitation: {e}");
            return;
        }

        let mut guard = self.pending_callbacks.write().await;
        guard.retain(|_, v| v.created_at.elapsed() < PENDING_TTL);
        guard.insert(
            request_id.to_string(),
            PendingCallback {
                request_id: request_id.to_string(),
                chat_id,
                values: vec!["true".to_string(), "false".to_string()],
                created_at: Instant::now(),
            },
        );
    }

    /// Send a text/secret prompt that expects the next text message as reply.
    async fn send_text_prompt(
        &self,
        bot: &Bot,
        chat_id: ChatId,
        request_id: &str,
        header: &str,
        placeholder: Option<&str>,
    ) {
        let mut msg = header.to_string();
        if let Some(ph) = placeholder {
            let _ = write!(msg, "\n\n<i>Hint: {}</i>", html_escape(ph));
        }
        msg.push_str("\n\nPlease type your response:");

        if let Err(e) = bot
            .send_message(chat_id, &msg)
            .parse_mode(ParseMode::Html)
            .await
        {
            warn!("Failed to send elicitation: {e}");
            return;
        }

        let mut guard = self.pending_text_replies.write().await;
        guard.retain(|_, v| v.created_at.elapsed() < PENDING_TTL);
        guard.insert(
            chat_id,
            PendingTextReply {
                request_id: request_id.to_string(),
                created_at: Instant::now(),
            },
        );
    }

    /// Handle a text message as an elicitation response.
    ///
    /// Returns `true` if the message was consumed as an elicitation reply.
    pub async fn handle_text_reply(
        &self,
        chat_id: ChatId,
        text: &str,
        daemon: &DaemonClient,
        sessions: &SessionMap,
    ) -> bool {
        let pending = self.pending_text_replies.write().await.remove(&chat_id);
        let Some(pending) = pending else {
            return false;
        };

        // From this point on, always return true — the pending entry was
        // consumed, so this message must not be treated as a normal bot message.
        let Some(session_id) = sessions.get_session_id(chat_id).await else {
            warn!("Elicitation reply consumed but no session for chat {chat_id}");
            return true;
        };

        let Ok(request_id_uuid) = uuid::Uuid::parse_str(&pending.request_id) else {
            warn!("Elicitation reply consumed but invalid request_id");
            return true;
        };

        let response = ElicitationResponse::submit(
            request_id_uuid,
            serde_json::Value::String(text.to_string()),
        );

        if let Err(e) = daemon
            .send_elicitation(&session_id, &pending.request_id, response)
            .await
        {
            warn!("Failed to send elicitation response: {e}");
        }

        true
    }

    /// Handle a callback query from an elicitation button press.
    ///
    /// Callback format: `eli:{prefix}:{index|yes|no|cancel}`
    pub async fn handle_callback(
        &self,
        bot: &Bot,
        query: &CallbackQuery,
        daemon: &DaemonClient,
        sessions: &SessionMap,
    ) -> bool {
        let data = match query.data.as_ref() {
            Some(d) if d.starts_with("eli:") => d,
            _ => return false,
        };

        let parts: Vec<&str> = data.splitn(3, ':').collect();
        if parts.len() != 3 {
            return false;
        }

        let prefix = parts[1];
        let action = parts[2];

        let pending = self.pending_callbacks.write().await.remove(prefix);
        let Some(pending) = pending else {
            let _ = bot.answer_callback_query(&query.id).text("Expired").await;
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

        let Some(response) = build_elicitation_response(action, request_id_uuid, &pending.values)
        else {
            let _ = bot
                .answer_callback_query(&query.id)
                .text("Unknown action")
                .await;
            return true;
        };

        if let Err(e) = daemon
            .send_elicitation(&session_id, &pending.request_id, response)
            .await
        {
            warn!("Failed to send elicitation response: {e}");
        }

        let _ = bot.answer_callback_query(&query.id).text("Submitted").await;

        // Remove keyboard from message.
        if let Some(msg) = &query.message {
            let _ = bot
                .edit_message_reply_markup(pending.chat_id, msg.id())
                .reply_markup(InlineKeyboardMarkup::new(
                    Vec::<Vec<InlineKeyboardButton>>::new(),
                ))
                .await;
        }

        true
    }
}

/// Build an `ElicitationResponse` from a callback action string.
fn build_elicitation_response(
    action: &str,
    request_id: uuid::Uuid,
    values: &[String],
) -> Option<ElicitationResponse> {
    match action {
        "cancel" => Some(ElicitationResponse::cancel(request_id)),
        "yes" => Some(ElicitationResponse::submit(
            request_id,
            serde_json::Value::Bool(true),
        )),
        "no" => Some(ElicitationResponse::submit(
            request_id,
            serde_json::Value::Bool(false),
        )),
        _ => {
            if let Ok(idx) = action.parse::<usize>() {
                values.get(idx).map(|value| {
                    ElicitationResponse::submit(
                        request_id,
                        serde_json::Value::String(value.clone()),
                    )
                })
            } else {
                None
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_response_cancel() {
        let id = uuid::Uuid::new_v4();
        let resp = build_elicitation_response("cancel", id, &[]).unwrap();
        assert_eq!(resp.request_id, id);
        assert!(matches!(
            resp.action,
            astrid_core::ElicitationAction::Cancel
        ));
    }

    #[test]
    fn build_response_yes() {
        let id = uuid::Uuid::new_v4();
        let resp = build_elicitation_response("yes", id, &[]).unwrap();
        if let astrid_core::ElicitationAction::Submit { value } = &resp.action {
            assert_eq!(*value, serde_json::Value::Bool(true));
        } else {
            panic!("Expected Submit action");
        }
    }

    #[test]
    fn build_response_no() {
        let id = uuid::Uuid::new_v4();
        let resp = build_elicitation_response("no", id, &[]).unwrap();
        if let astrid_core::ElicitationAction::Submit { value } = &resp.action {
            assert_eq!(*value, serde_json::Value::Bool(false));
        } else {
            panic!("Expected Submit action");
        }
    }

    #[test]
    fn build_response_index_valid() {
        let id = uuid::Uuid::new_v4();
        let values = vec!["opt_a".to_string(), "opt_b".to_string()];
        let resp = build_elicitation_response("1", id, &values).unwrap();
        if let astrid_core::ElicitationAction::Submit { value } = &resp.action {
            assert_eq!(*value, serde_json::Value::String("opt_b".to_string()));
        } else {
            panic!("Expected Submit action");
        }
    }

    #[test]
    fn build_response_index_out_of_bounds() {
        let id = uuid::Uuid::new_v4();
        let values = vec!["opt_a".to_string()];
        assert!(build_elicitation_response("5", id, &values).is_none());
    }

    #[test]
    fn build_response_unknown_action() {
        let id = uuid::Uuid::new_v4();
        assert!(build_elicitation_response("foobar", id, &[]).is_none());
    }
}
