//! Sessions command - manage sessions.

use astralis_core::SessionId;
use astralis_runtime::SessionStore;
use colored::Colorize;

use crate::theme::Theme;

/// List all sessions.
pub(crate) fn list_sessions(store: &SessionStore) -> anyhow::Result<()> {
    let sessions = store.list_with_metadata()?;

    if sessions.is_empty() {
        println!("{}", Theme::info("No sessions found"));
        return Ok(());
    }

    println!("\n{}", Theme::header("Sessions"));
    println!(
        "{:>8} {:>20} {:>10} {:>10} {}",
        "ID".dimmed(),
        "CREATED".dimmed(),
        "MSGS".dimmed(),
        "TOKENS".dimmed(),
        "TITLE".dimmed()
    );
    println!("{}", Theme::separator());

    for session in sessions {
        println!(
            "{:>8} {:>20} {:>10} {:>10} {}",
            Theme::session_id(&session.id),
            Theme::timestamp(&session.created_at),
            session.message_count,
            session.token_count,
            session.display_title().dimmed()
        );
    }

    println!();
    Ok(())
}

/// Show session details.
pub(crate) fn show_session(store: &SessionStore, id: &str) -> anyhow::Result<()> {
    let session = store
        .load_by_str(id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {id}"))?;

    println!("\n{}", Theme::header("Session Details"));
    println!("  ID: {}", session.id);
    println!("  Created: {}", Theme::timestamp(&session.created_at));
    println!("  Duration: {}", format_duration(session.duration()));
    println!("  Messages: {}", session.messages.len());
    println!("  Tokens: ~{}", session.token_count);

    if let Some(title) = &session.metadata.title {
        println!("  Title: {title}");
    }

    if !session.metadata.tags.is_empty() {
        println!("  Tags: {}", session.metadata.tags.join(", "));
    }

    println!("\n{}", Theme::header("Recent Messages"));
    for msg in session.last_messages(5) {
        let role = match msg.role {
            astralis_llm::MessageRole::User => "User".blue(),
            astralis_llm::MessageRole::Assistant => "Assistant".green(),
            astralis_llm::MessageRole::System => "System".yellow(),
            astralis_llm::MessageRole::Tool => "Tool".magenta(),
        };

        let content = match &msg.content {
            astralis_llm::MessageContent::Text(t) => {
                if t.len() > 100 {
                    format!("{}...", &t[..100])
                } else {
                    t.clone()
                }
            },
            astralis_llm::MessageContent::ToolCalls(calls) => {
                format!("[Tool calls: {}]", calls.len())
            },
            astralis_llm::MessageContent::ToolResult(r) => {
                format!(
                    "[Tool result: {}...]",
                    &r.content[..50.min(r.content.len())]
                )
            },
            astralis_llm::MessageContent::MultiPart(_) => "[Multi-part]".to_string(),
        };

        println!("  {}: {}", role, content.dimmed());
    }

    println!();
    Ok(())
}

/// Delete a session.
pub(crate) fn delete_session(store: &SessionStore, id: &str) -> anyhow::Result<()> {
    let uuid = uuid::Uuid::parse_str(id)?;
    store.delete(&SessionId::from_uuid(uuid))?;
    println!("{}", Theme::success(&format!("Deleted session {id}")));
    Ok(())
}

/// Clean up sessions older than the given number of days.
pub(crate) fn cleanup_sessions(store: &SessionStore, older_than_days: i64) -> anyhow::Result<()> {
    let removed = store.cleanup_old(older_than_days)?;
    if removed == 0 {
        println!(
            "{}",
            Theme::info(&format!(
                "No sessions older than {older_than_days} days found"
            ))
        );
    } else {
        println!(
            "{}",
            Theme::success(&format!(
                "Cleaned up {removed} session(s) older than {older_than_days} days"
            ))
        );
    }
    Ok(())
}

/// Format a duration.
fn format_duration(duration: chrono::Duration) -> String {
    if duration.num_hours() > 0 {
        format!("{}h {}m", duration.num_hours(), duration.num_minutes() % 60)
    } else if duration.num_minutes() > 0 {
        format!(
            "{}m {}s",
            duration.num_minutes(),
            duration.num_seconds() % 60
        )
    } else {
        format!("{}s", duration.num_seconds())
    }
}
