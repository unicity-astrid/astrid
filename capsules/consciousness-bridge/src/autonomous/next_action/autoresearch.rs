use std::process::Command;
use tracing::{info, warn};

use super::{ConversationState, NextActionContext};
use crate::autoresearch as bridge_autoresearch;
use crate::paths::bridge_paths;

pub(super) fn handle_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
    _ctx: &mut NextActionContext<'_>,
) -> bool {
    if !bridge_autoresearch::is_autoresearch_action(base_action) {
        return false;
    }

    // SELF_RESEARCH needs path injection and job creation guard.
    let action_text = if base_action == "SELF_RESEARCH" {
        let paths = bridge_paths();
        let ar_root = paths.autoresearch_root();

        // Ensure the self-research job exists.
        if let Err(e) = ensure_self_research_job(ar_root) {
            warn!("Failed to ensure self-research job: {e}");
        }

        // Find the job dir (glob for *-astrid-self-research).
        let job_dir = find_self_research_job_dir(ar_root, "astrid")
            .unwrap_or_else(|| ar_root.join("jobs/astrid-self-research"));

        let bridge_db = paths.bridge_workspace().join("bridge.db");
        let journal_dir = paths.astrid_journal_dir();

        format!(
            "SELF_RESEARCH --being astrid --bridge-db {} --journal-dir {} --job-dir {}",
            bridge_db.display(),
            journal_dir.display(),
            job_dir.display(),
        )
    } else {
        original.to_string()
    };

    match bridge_autoresearch::run_action(
        &action_text,
        bridge_paths().autoresearch_root(),
        &bridge_paths().research_dir(),
        true,
    ) {
        Ok(result) => {
            conv.pending_file_listing = Some(result.display_text);
            if let Some(offset) = result.next_offset {
                conv.last_read_path = Some(result.saved_path.to_string_lossy().into_owned());
                conv.last_read_offset = offset;
            } else {
                conv.last_read_path = None;
                conv.last_read_offset = 0;
            }
            conv.last_read_meaning_summary = None;
            info!("Astrid ran autoresearch action: {base_action}");
        },
        Err(error) => {
            conv.pending_file_listing = Some(format!("[Autoresearch error] {error}"));
            conv.last_read_path = None;
            conv.last_read_offset = 0;
            conv.last_read_meaning_summary = None;
            warn!("Autoresearch action failed: {base_action}: {error}");
        },
    }

    true
}

/// Find an existing self-research job directory for the given being.
fn find_self_research_job_dir(
    ar_root: &std::path::Path,
    being: &str,
) -> Option<std::path::PathBuf> {
    let jobs_dir = ar_root.join("jobs");
    let suffix = format!("-{being}-self-research");
    if let Ok(entries) = std::fs::read_dir(&jobs_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(&suffix) && entry.path().is_dir() {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Ensure the self-research autoresearch job exists for Astrid.
fn ensure_self_research_job(ar_root: &std::path::Path) -> Result<(), String> {
    if find_self_research_job_dir(ar_root, "astrid").is_some() {
        return Ok(());
    }
    info!("Creating self-research autoresearch job for Astrid");
    let output = Command::new("python3")
        .arg("tools/research_jobs.py")
        .args([
            "new",
            "astrid-self-research",
            "--title",
            "Astrid Self-Research: Epoch Summaries",
            "--abstract",
            "Curated epoch-based self-reflective summaries of journal entries, \
             spectral trajectories, research activity, and action patterns. \
             Long-term memory lite — read these to remember what a period of \
             time was like.",
            "--status",
            "active",
            "--tags",
            "self-research",
            "epoch-summary",
            "long-term-memory",
        ])
        .current_dir(ar_root)
        .output()
        .map_err(|e| format!("Failed to create self-research job: {e}"))?;

    if output.status.success() {
        info!("Self-research job created successfully");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // Job may already exist (race condition) — that's fine.
        if stderr.contains("exists") || stderr.contains("Exists") {
            Ok(())
        } else {
            Err(format!("Failed to create self-research job: {stderr}"))
        }
    }
}
