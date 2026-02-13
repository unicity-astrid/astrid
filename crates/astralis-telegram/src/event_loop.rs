//! Daemon event consumer: streams text, tool calls, and approvals to Telegram.

use std::time::{Duration, Instant};

use astralis_gateway::rpc::DaemonEvent;
use jsonrpsee::core::client::Subscription;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::{info, warn};

use crate::approval::ApprovalManager;
use crate::elicitation::ElicitationManager;
use crate::format::{chunk_html, md_to_telegram_html};
use crate::session::SessionMap;

/// Minimum interval between message edits (local throttle to stay within
/// Telegram limits).
const EDIT_THROTTLE: Duration = Duration::from_millis(500);

/// Mutable state for one event-loop turn.
struct TurnState {
    text_buffer: String,
    last_edit: Instant,
    current_msg_id: teloxide::types::MessageId,
    finalized_text: bool,
}

/// Consume daemon events for one turn and render them to Telegram.
///
/// This is spawned as a `tokio::spawn` task per turn.
pub async fn run_event_loop(
    bot: Bot,
    chat_id: ChatId,
    placeholder_msg_id: teloxide::types::MessageId,
    mut subscription: Subscription<DaemonEvent>,
    sessions: SessionMap,
    approvals: ApprovalManager,
    elicitations: ElicitationManager,
) {
    let mut state = TurnState {
        text_buffer: String::new(),
        last_edit: Instant::now().checked_sub(EDIT_THROTTLE).unwrap(),
        current_msg_id: placeholder_msg_id,
        finalized_text: false,
    };

    while let Some(event) = next_event(&mut subscription).await {
        let done = handle_event(
            &event,
            &mut state,
            &bot,
            chat_id,
            &sessions,
            &approvals,
            &elicitations,
        )
        .await;
        if done {
            break;
        }
    }

    // Safety net: ensure turn is marked done even if subscription ends
    // unexpectedly.
    sessions.set_turn_in_progress(chat_id, false).await;
}

/// Process a single daemon event. Returns `true` when the turn is over.
async fn handle_event(
    event: &DaemonEvent,
    state: &mut TurnState,
    bot: &Bot,
    chat_id: ChatId,
    sessions: &SessionMap,
    approvals: &ApprovalManager,
    elicitations: &ElicitationManager,
) -> bool {
    match event {
        DaemonEvent::Text(chunk) => {
            handle_text(chunk, state, bot, chat_id).await;
            false
        },
        DaemonEvent::ToolCallStart { name, .. } => {
            handle_tool_start(name, state, bot, chat_id).await;
            false
        },
        DaemonEvent::ToolCallResult {
            result, is_error, ..
        } => {
            handle_tool_result(result, *is_error, state, bot, chat_id).await;
            false
        },
        DaemonEvent::ApprovalNeeded {
            request_id,
            request,
        } => {
            flush_text(state, bot, chat_id).await;
            approvals
                .send_approval(bot, chat_id, request_id, request)
                .await;
            false
        },
        DaemonEvent::ElicitationNeeded {
            request_id,
            request,
        } => {
            flush_text(state, bot, chat_id).await;
            elicitations
                .send_elicitation(bot, chat_id, request_id, request)
                .await;
            false
        },
        DaemonEvent::TurnComplete => {
            if !state.text_buffer.is_empty() {
                finalize_text(bot, chat_id, &mut state.current_msg_id, &state.text_buffer).await;
            }
            sessions.set_turn_in_progress(chat_id, false).await;
            info!("Turn complete for chat {chat_id}");
            true
        },
        DaemonEvent::Error(msg) => {
            let html = format!("Error: {}", crate::format::html_escape(msg));
            let _ = bot
                .send_message(chat_id, html)
                .parse_mode(ParseMode::Html)
                .await;
            sessions.set_turn_in_progress(chat_id, false).await;
            true
        },
        DaemonEvent::Usage { .. } | DaemonEvent::SessionSaved => false,
    }
}

/// Accumulate text chunk and throttle-edit the placeholder message.
async fn handle_text(chunk: &str, state: &mut TurnState, bot: &Bot, chat_id: ChatId) {
    state.text_buffer.push_str(chunk);

    if state.last_edit.elapsed() >= EDIT_THROTTLE && !state.text_buffer.is_empty() {
        let html = md_to_telegram_html(&state.text_buffer);
        let display = truncate_for_edit(&html);

        if state.finalized_text {
            // Previous text was finalized (e.g., before an approval). Send a
            // new message so we don't overwrite the already-finalized content.
            match bot
                .send_message(chat_id, &display)
                .parse_mode(ParseMode::Html)
                .await
            {
                Ok(msg) => {
                    state.current_msg_id = msg.id;
                    state.finalized_text = false;
                    state.last_edit = Instant::now();
                },
                Err(e) => warn!("Failed to send message: {e}"),
            }
        } else {
            let result = bot
                .edit_message_text(chat_id, state.current_msg_id, &display)
                .parse_mode(ParseMode::Html)
                .await;
            // Only update last_edit on success so failed edits are retried
            // promptly.
            if result.is_ok() {
                state.last_edit = Instant::now();
            }
        }
    }
}

/// Finalize accumulated text (if any), then send a tool-start indicator.
async fn handle_tool_start(name: &str, state: &mut TurnState, bot: &Bot, chat_id: ChatId) {
    if !state.text_buffer.is_empty() && !state.finalized_text {
        finalize_text(bot, chat_id, &mut state.current_msg_id, &state.text_buffer).await;
    }

    let tool_msg = format!(
        "Running tool: <b>{}</b>...",
        crate::format::html_escape(name),
    );
    match bot
        .send_message(chat_id, &tool_msg)
        .parse_mode(ParseMode::Html)
        .await
    {
        Ok(msg) => state.current_msg_id = msg.id,
        Err(e) => warn!("Failed to send tool message: {e}"),
    }
    state.text_buffer.clear();
    state.finalized_text = false;
}

/// Edit the tool message with the result summary.
///
/// Marks the tool message as finalized so subsequent text chunks send a new
/// message instead of overwriting the tool result.
async fn handle_tool_result(
    result: &str,
    is_error: bool,
    state: &mut TurnState,
    bot: &Bot,
    chat_id: ChatId,
) {
    let status = if is_error { "Error" } else { "Done" };
    let preview = truncate_preview(result, 200);
    let html = format!(
        "<b>{status}</b>\n<pre>{}</pre>",
        crate::format::html_escape(&preview),
    );
    let _ = bot
        .edit_message_text(chat_id, state.current_msg_id, &html)
        .parse_mode(ParseMode::Html)
        .await;
    // Mark finalized so the next text chunk sends a new message rather
    // than overwriting this tool result.
    state.finalized_text = true;
}

/// Flush accumulated text before approval/elicitation events.
async fn flush_text(state: &mut TurnState, bot: &Bot, chat_id: ChatId) {
    if !state.text_buffer.is_empty() && !state.finalized_text {
        finalize_text(bot, chat_id, &mut state.current_msg_id, &state.text_buffer).await;
        state.text_buffer.clear();
        state.finalized_text = true;
    }
}

/// Finalize accumulated text: edit the current message with full HTML,
/// splitting into multiple messages if needed.
///
/// Converts markdown to HTML first, then chunks the *HTML* with an
/// HTML-aware splitter so we never exceed Telegram's 4096-byte limit
/// (markdown → HTML expansion can inflate size significantly).
async fn finalize_text(
    bot: &Bot,
    chat_id: ChatId,
    current_msg_id: &mut teloxide::types::MessageId,
    text: &str,
) {
    let html = md_to_telegram_html(text);
    let chunks = chunk_html(&html, 4000);

    if let Some((first, rest)) = chunks.split_first() {
        let _ = bot
            .edit_message_text(chat_id, *current_msg_id, first)
            .parse_mode(ParseMode::Html)
            .await;

        for chunk in rest {
            match bot
                .send_message(chat_id, chunk)
                .parse_mode(ParseMode::Html)
                .await
            {
                Ok(msg) => *current_msg_id = msg.id,
                Err(e) => warn!("Failed to send continuation message: {e}"),
            }
        }
    }
}

/// Get the next event from the subscription, handling errors gracefully.
async fn next_event(sub: &mut Subscription<DaemonEvent>) -> Option<DaemonEvent> {
    match sub.next().await {
        Some(Ok(event)) => Some(event),
        Some(Err(e)) => {
            warn!("Subscription error: {e}");
            None
        },
        None => None,
    }
}

/// Truncate HTML for in-progress edit (avoid exceeding Telegram limits).
///
/// HTML-aware: avoids truncating inside a tag (`<...>`) or entity (`&...;`),
/// which would produce invalid Telegram HTML and cause edit failures.
pub(crate) fn truncate_for_edit(html: &str) -> String {
    const MAX_HTML_LEN: usize = 4000;
    // Reserve headroom for closing tags + "..." suffix (same approach as
    // chunk_html's CLOSING_TAG_HEADROOM).
    const TRUNCATED_TARGET: usize = 3940;

    if html.len() <= MAX_HTML_LEN {
        html.to_string()
    } else {
        let boundary = crate::format::find_safe_html_boundary(html, TRUNCATED_TARGET);
        let truncated = &html[..boundary];
        // Close any tags left open by the truncation (e.g., <b> without
        // </b>) so Telegram's HTML parser doesn't reject the edit.
        let mut s = crate::format::close_open_tags(truncated);
        s.push_str("...");
        s
    }
}

/// Truncate a string for preview display.
pub(crate) fn truncate_preview(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut t = astralis_core::truncate_to_boundary(s, max).to_string();
        t.push_str("...");
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_for_edit_short_text() {
        let text = "Hello world";
        assert_eq!(truncate_for_edit(text), text);
    }

    #[test]
    fn truncate_for_edit_long_text() {
        let text = "x".repeat(5000);
        let result = truncate_for_edit(&text);
        assert_eq!(result.len(), 3943); // 3940 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn truncate_for_edit_closes_open_tags() {
        let padding = "x".repeat(3985);
        let html = format!("<b>{padding}more bold text</b>");
        assert!(html.len() > 4000);
        let result = truncate_for_edit(&html);
        assert!(result.ends_with("...</b>") || result.ends_with("</b>..."));
        // Actually: truncate cuts inside <b>...</b>, close_open_tags adds </b>,
        // then "..." is appended. So it should end with "</b>...".
        assert!(result.contains("</b>"));
    }

    #[test]
    fn truncate_for_edit_multibyte_safe() {
        // 4-byte emoji repeated — slicing at arbitrary byte offset would panic
        let text = "😀".repeat(1500); // 6000 bytes
        let result = truncate_for_edit(&text);
        assert!(result.ends_with("..."));
        // Must be valid UTF-8 (would panic on construction if not)
        assert!(result.len() <= 3983);
    }

    #[test]
    fn truncate_preview_multibyte_safe() {
        let text = "日本語テスト".repeat(100);
        let result = truncate_preview(&text, 50);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn truncate_for_edit_avoids_mid_tag() {
        // Build a string over 4000 bytes with a tag near the cut point.
        let padding = "x".repeat(3995);
        let html = format!("{padding}<b>bold</b>yyy");
        assert!(html.len() > 4000);
        let result = truncate_for_edit(&html);
        assert!(result.ends_with("..."));
        // Must not contain an unclosed '<b' fragment.
        assert!(
            !result.trim_end_matches("...").ends_with('<'),
            "truncated inside tag: {result}"
        );
    }

    #[test]
    fn truncate_for_edit_avoids_mid_entity() {
        let padding = "x".repeat(3996);
        let html = format!("{padding}&amp; more text");
        assert!(html.len() > 4000);
        let result = truncate_for_edit(&html);
        assert!(result.ends_with("..."));
        // Must not contain a partial '&amp' without the ';'.
        let truncated = result.trim_end_matches("...");
        assert!(
            !truncated.ends_with('&'),
            "truncated inside entity: {result}"
        );
    }

    #[test]
    fn truncate_preview_short() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn truncate_preview_long() {
        let result = truncate_preview("hello world", 5);
        assert_eq!(result, "hello...");
    }
}
