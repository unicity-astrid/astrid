//! Audit command - view and verify audit logs.

use astrid_audit::AuditLog;
use astrid_core::SessionId;
use colored::Colorize;

use crate::theme::Theme;

/// List audit sessions.
pub(crate) fn list_audit_sessions(log: &AuditLog) -> anyhow::Result<()> {
    let sessions = log.list_sessions()?;

    if sessions.is_empty() {
        println!("{}", Theme::info("No audit entries"));
        return Ok(());
    }

    println!("\n{}", Theme::header("Audit Sessions"));
    println!("{:>8} {:>10}", "SESSION".dimmed(), "ENTRIES".dimmed());
    println!("{}", Theme::separator());

    for session_id in sessions {
        let count = log.count_session(&session_id)?;
        println!(
            "{:>8} {:>10}",
            Theme::session_id(&session_id.0.to_string()),
            count
        );
    }

    println!();
    Ok(())
}

/// Show audit entries for a session.
pub(crate) fn show_audit_entries(log: &AuditLog, session_id: &str) -> anyhow::Result<()> {
    let uuid = uuid::Uuid::parse_str(session_id)?;
    let entries = log.get_session_entries(&SessionId::from_uuid(uuid))?;

    if entries.is_empty() {
        println!("{}", Theme::info("No entries for this session"));
        return Ok(());
    }

    println!("\n{}", Theme::header("Audit Entries"));
    println!(
        "{:>20} {:>25} {}",
        "TIMESTAMP".dimmed(),
        "ACTION".dimmed(),
        "RESULT".dimmed()
    );
    println!("{}", Theme::separator());

    for entry in entries {
        let timestamp = Theme::timestamp(&entry.timestamp.0);
        let action = entry.action.description();
        let result = if matches!(entry.outcome, astrid_audit::AuditOutcome::Success { .. }) {
            "OK".green().to_string()
        } else {
            "FAIL".red().to_string()
        };

        println!("{timestamp:>20} {action:>25} {result}");
    }

    println!();
    Ok(())
}

/// Verify audit chain integrity.
pub(crate) fn verify_audit_chain(log: &AuditLog, session_id: Option<&str>) -> anyhow::Result<()> {
    if let Some(id) = session_id {
        // Verify specific session
        let uuid = uuid::Uuid::parse_str(id)?;
        let result = log.verify_chain(&SessionId::from_uuid(uuid))?;

        if result.valid {
            println!(
                "{}",
                Theme::success(&format!(
                    "Session {} verified: {} entries, no issues",
                    &id[..8],
                    result.entries_verified
                ))
            );
        } else {
            println!(
                "{}",
                Theme::error(&format!(
                    "Session {} has {} issues:",
                    &id[..8],
                    result.issues.len()
                ))
            );
            for issue in &result.issues {
                println!("  - {issue}");
            }
        }
    } else {
        // Verify all sessions
        let results = log.verify_all()?;

        let valid_count = results.iter().filter(|(_, r)| r.valid).count();
        let total_count = results.len();

        if valid_count == total_count {
            println!(
                "{}",
                Theme::success(&format!("All {total_count} sessions verified"))
            );
        } else {
            println!(
                "{}",
                Theme::warning(&format!("{valid_count}/{total_count} sessions valid"))
            );

            for (session_id, result) in &results {
                if !result.valid {
                    println!(
                        "\n{}",
                        Theme::error(&format!(
                            "Session {} has {} issues:",
                            &session_id.0.to_string()[..8],
                            result.issues.len()
                        ))
                    );
                    for issue in &result.issues {
                        println!("  - {issue}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Show audit statistics.
pub(crate) fn show_audit_stats(log: &AuditLog) -> anyhow::Result<()> {
    let total_entries = log.count()?;
    let sessions = log.list_sessions()?;

    println!("\n{}", Theme::header("Audit Statistics"));
    println!("  Total entries: {total_entries}");
    println!("  Total sessions: {}", sessions.len());
    println!(
        "  Runtime key: {}",
        hex::encode(&log.runtime_public_key().as_bytes()[..8])
    );

    // Verify all
    let results = log.verify_all()?;
    let valid_count = results.iter().filter(|(_, r)| r.valid).count();

    if valid_count == results.len() {
        println!("  Integrity: {}", "OK".green());
    } else {
        println!(
            "  Integrity: {} ({}/{} valid)",
            "ISSUES".red(),
            valid_count,
            results.len()
        );
    }

    println!();
    Ok(())
}
