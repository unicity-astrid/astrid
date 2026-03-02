//! Sessions command - manage sessions.

use astrid_core::SessionId;
use colored::Colorize;

use crate::daemon_client::DaemonClient;
use crate::theme::Theme;

/// List all sessions.
pub(crate) async fn list_sessions(client: &DaemonClient) -> anyhow::Result<()> {
    let sessions = client.list_sessions(None).await?;

    if sessions.is_empty() {
        println!("{}", Theme::info("No sessions found"));
        return Ok(());
    }

    println!("\n{}", Theme::header("Active Sessions"));
    println!(
        "{:>8} {:>20} {:>10} {}",
        "ID".dimmed(),
        "CREATED".dimmed(),
        "MSGS".dimmed(),
        "WORKSPACE".dimmed()
    );
    println!("{}", Theme::separator());

    for session in sessions {
        let ws_display = session
            .workspace
            .map_or_else(|| "[Global]".to_string(), |p| p.display().to_string());

        let id_str = session.id.0.to_string();
        println!(
            "{:>8} {:>20} {:>10} {}",
            Theme::session_id(&id_str),
            Theme::timestamp(&session.created_at),
            session.message_count,
            ws_display.dimmed()
        );
    }

    println!();
    Ok(())
}

/// Delete a session.
pub(crate) async fn delete_session(client: &DaemonClient, id: &str) -> anyhow::Result<()> {
    let uuid = uuid::Uuid::parse_str(id)?;
    client.end_session(&SessionId::from_uuid(uuid)).await?;
    println!("{}", Theme::success(&format!("Ended session {id}")));
    Ok(())
}
