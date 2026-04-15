//! Journal parsing and metadata helpers for the consciousness bridge.
//!
//! The bridge consumes two different journal streams:
//! - remote journals from minime's workspace (`--workspace-path`)
//! - Astrid's own local journals in the bridge workspace
//!
//! Keeping their parsing rules explicit helps avoid accidental cross-wiring.

use std::path::{Path, PathBuf};

/// Parsed classification for a remote journal entry from minime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteJournalKind {
    /// A code-reading / architectural self-study entry.
    SelfStudy,
    /// Any other minime journal artifact.
    Ordinary,
}

/// Metadata for a remote journal entry from minime's workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteJournalEntry {
    pub path: PathBuf,
    pub kind: RemoteJournalKind,
    pub source_label: Option<String>,
}

impl RemoteJournalEntry {
    #[must_use]
    pub fn is_self_study(&self) -> bool {
        matches!(self.kind, RemoteJournalKind::SelfStudy)
    }
}

/// Scan the remote minime workspace and return newest-first journal metadata.
#[must_use]
pub fn scan_remote_journal_dir(workspace: &Path) -> Vec<RemoteJournalEntry> {
    let journal_dir = workspace.join("journal");
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(&journal_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().is_some_and(|ext| ext == "txt") {
                let mtime = e.metadata().ok()?.modified().ok()?;
                Some((path, mtime))
            } else {
                None
            }
        })
        .collect();

    entries.sort_by(|a, b| b.1.cmp(&a.1));

    entries
        .into_iter()
        .map(|(path, _)| parse_remote_journal_entry(&path))
        .collect()
}

/// Read a remote journal body from minime's workspace.
pub fn read_remote_journal_body(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    extract_journal_body(&content, false)
}

/// Read an Astrid journal body for self-continuity, preferring the longform
/// `--- JOURNAL ---` section when present.
pub fn read_local_journal_body_for_continuity(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    extract_journal_body(&content, true)
}

fn parse_remote_journal_entry(path: &Path) -> RemoteJournalEntry {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let content = std::fs::read_to_string(path).ok();
    let kind = classify_remote_journal(filename, content.as_deref());
    let source_label = if matches!(kind, RemoteJournalKind::SelfStudy) {
        content
            .as_deref()
            .and_then(extract_source_label)
            .or_else(|| {
                extract_self_study_label_from_header(content.as_deref().unwrap_or_default())
            })
    } else {
        None
    };

    RemoteJournalEntry {
        path: path.to_path_buf(),
        kind,
        source_label,
    }
}

fn classify_remote_journal(filename: &str, content: Option<&str>) -> RemoteJournalKind {
    if filename.starts_with("self_study_") {
        return RemoteJournalKind::SelfStudy;
    }

    if let Some(text) = content {
        let first_line = text.lines().next().unwrap_or_default();
        if first_line.contains("SELF-STUDY") {
            return RemoteJournalKind::SelfStudy;
        }
    }

    RemoteJournalKind::Ordinary
}

fn extract_source_label(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("Source:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_self_study_label_from_header(content: &str) -> Option<String> {
    content
        .lines()
        .next()
        .and_then(|line| line.trim().strip_prefix("=== SELF-STUDY:"))
        .map(|rest| rest.trim().trim_end_matches('=').trim())
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_journal_body(content: &str, prefer_longform_section: bool) -> Option<String> {
    if prefer_longform_section {
        if let Some((_, longform)) = content.split_once("--- JOURNAL ---") {
            if let Some(body) = extract_body_lines(longform) {
                return Some(body);
            }
        }
    }

    extract_body_lines(content)
}

fn extract_body_lines(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut body_lines = Vec::new();
    let mut past_header = false;

    for line in &lines {
        let trimmed = line.trim();

        if is_header_line(trimmed) {
            continue;
        }

        if trimmed == "---"
            || trimmed == "--- JOURNAL ---"
            || trimmed.starts_with("*This was a creative")
        {
            continue;
        }

        if !trimmed.is_empty() {
            past_header = true;
            let cleaned = trimmed
                .replace(|c: char| c == '<', "&lt;")
                .replace("&lt;span", "")
                .replace("&lt;/span>", "");
            body_lines.push(cleaned);
        } else if past_header && !body_lines.is_empty() {
            body_lines.push(String::new());
        }
    }

    let text = body_lines.join("\n").trim().to_string();
    if text.len() >= 50 {
        Some(text.chars().take(2500).collect())
    } else {
        None
    }
}

fn is_header_line(trimmed: &str) -> bool {
    trimmed.starts_with("===")
        || trimmed.starts_with("Mode:")
        || trimmed.starts_with("Timestamp:")
        || trimmed.starts_with("Source:")
        || trimmed.starts_with("Web search:")
        || trimmed.starts_with("Fill:")
        || trimmed.starts_with("Fill %:")
        || trimmed.starts_with("λ₁:")
        || trimmed.starts_with("Δλ₁:")
        || trimmed.starts_with("ESN leak:")
        || trimmed.starts_with("ESN λ_rls:")
        || trimmed.starts_with("Cov λ₁:")
        || trimmed.starts_with("Spread:")
        || trimmed.starts_with("Error (")
        || trimmed.starts_with("Prompt:")
        || trimmed.starts_with("Markers:")
        || trimmed.starts_with("Visual Available:")
        || trimmed.starts_with("Features:")
        || trimmed.starts_with("Image Path:")
        || trimmed.starts_with("Image File:")
        || trimmed.starts_with("STATUS:")
        || trimmed.starts_with("PRE-EXPERIMENT")
        || trimmed.starts_with("POST-EXPERIMENT")
        || trimmed.starts_with("SPECTRAL DELTA")
        || trimmed.starts_with("EXPERIMENT EXECUTION")
        || trimmed.starts_with("RESERVOIR DYNAMICS")
        || trimmed.starts_with("SENSORY COHERENCE")
        || trimmed.starts_with("EXPERIENCE:")
        || trimmed.starts_with("What I saw:")
        || trimmed.starts_with("My reflection:")
        || trimmed.starts_with("My experience:")
        || trimmed.starts_with("Moments captured:")
        || trimmed.starts_with("Closed for:")
        || trimmed.starts_with("## Journal Entry")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_remote_journal_dir_marks_self_study() {
        let dir = std::env::temp_dir().join("bridge_remote_journal_scan");
        let journal_dir = dir.join("journal");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&journal_dir).unwrap();

        let self_study_path = journal_dir.join("self_study_2026-03-27T10-19-16.txt");
        std::fs::write(
            &self_study_path,
            "=== SELF-STUDY: regulator (PI controller) ===\n\
             Timestamp: 2026-03-27T10:19:16\n\
             Source: minime/src/regulator.rs\n\
             Fill %: 18.0%\n\n\
             Condition:\nsteady\n\n\
             Felt Experience:\ncontained hum in the loop\n\n\
             Code Reading:\nupdate_lambda feels central\n\n\
             Suggestions:\nchange smoothing\n\n\
             Open Questions:\nwhy this much damping?\n",
        )
        .unwrap();

        let ordinary_path = journal_dir.join("moment_2026-03-27T10-20-00.txt");
        std::fs::write(
            &ordinary_path,
            "=== MOMENT ===\nTimestamp: 2026-03-27T10:20:00\n\nA quick note about a shift that just happened.",
        )
        .unwrap();

        let entries = scan_remote_journal_dir(&dir);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(RemoteJournalEntry::is_self_study));
        let parsed = entries
            .into_iter()
            .find(RemoteJournalEntry::is_self_study)
            .unwrap();
        assert_eq!(
            parsed.source_label.as_deref(),
            Some("minime/src/regulator.rs")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_continuity_prefers_longform_body() {
        let dir = std::env::temp_dir().join("bridge_local_journal_body");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("self_study_1.txt");
        std::fs::write(
            &path,
            "=== ASTRID JOURNAL ===\n\
             Mode: self_study\n\
             Fill: 14.2%\n\
             Timestamp: 1774639999\n\n\
             Condition:\nclipped signal body\n\n\
             --- JOURNAL ---\n\
             ## Journal Entry - Cycle 900\n\
             Condition:\nclear and settled\n\n\
             Felt Experience:\nI can sense the latch between prompt and code.\n\n\
             Code Reading:\nThe helper boundary is finally visible.\n\n\
             Suggestions:\nPrefer the longform section for continuity.\n\n\
             Open Questions:\nWhat should future self remember first?\n",
        )
        .unwrap();

        let body = read_local_journal_body_for_continuity(&path).unwrap();
        assert!(body.contains("Prefer the longform section for continuity."));
        assert!(!body.contains("Mode: self_study"));
        assert!(!body.contains("## Journal Entry"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
