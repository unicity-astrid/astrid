//! Budget-aware prompt assembly with overflow to disk.
//!
//! When the total content exceeds the character budget, lowest-priority
//! blocks are trimmed first and their overflow is written to a single file
//! that the existing READ_MORE infrastructure can serve back on demand.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// A labeled block of prompt content with its priority.
pub struct PromptBlock {
    /// Human-readable label (e.g. "spectral", "journal").
    pub label: &'static str,
    /// The full content for this block.
    pub content: String,
    /// Lower number = higher priority (trimmed last).
    pub priority: u8,
}

/// Metadata about content that was spilled to disk.
pub struct PromptOverflow {
    /// Path to the overflow file on disk.
    pub path: PathBuf,
    /// Character offset past the included portion (for READ_MORE).
    pub offset: usize,
    /// Human-readable summary of what overflowed.
    pub summary: String,
}

/// Structured report about how the prompt budget was applied.
#[derive(Debug, Clone, Serialize)]
pub struct PromptBudgetReport {
    pub budget: usize,
    pub total_before: usize,
    pub total_after: usize,
    pub trimmed_blocks: Vec<PromptTrimmedBlock>,
}

/// One block that was partially or fully trimmed.
#[derive(Debug, Clone, Serialize)]
pub struct PromptTrimmedBlock {
    pub label: String,
    pub original_chars: usize,
    pub kept_chars: usize,
    pub removed_chars: usize,
    pub fully_removed: bool,
}

/// Assemble blocks within a character budget.
///
/// Blocks are concatenated in their original order. If the total exceeds
/// `budget`, lowest-priority blocks are progressively trimmed and the
/// removed content is written to `overflow_dir/context_overflow_{ts}.txt`.
///
/// Each trimmed block gets a notice appended:
/// `[...N chars of {label} trimmed. NEXT: READ_MORE to see full context.]`
///
/// Returns the assembled text and optional overflow metadata.
pub fn assemble_within_budget(
    blocks: Vec<PromptBlock>,
    budget: usize,
    overflow_dir: &Path,
) -> (String, Option<PromptOverflow>, Option<PromptBudgetReport>) {
    // Filter out empty blocks and compute total.
    let blocks: Vec<PromptBlock> = blocks
        .into_iter()
        .filter(|b| !b.content.trim().is_empty())
        .collect();

    let total: usize = blocks.iter().map(|b| b.content.len()).sum();

    if total <= budget {
        // Everything fits — concatenate in order and return.
        let assembled = blocks
            .into_iter()
            .map(|b| b.content)
            .collect::<Vec<_>>()
            .join("\n");
        return (assembled, None, None);
    }

    // Need to trim. Build a priority-sorted index (highest priority number = trimmed first).
    let mut trim_order: Vec<usize> = (0..blocks.len()).collect();
    trim_order.sort_by(|&a, &b| blocks[b].priority.cmp(&blocks[a].priority));

    // Mutable copies of content for trimming.
    let mut contents: Vec<String> = blocks.iter().map(|b| b.content.clone()).collect();
    let mut overflow_sections: Vec<(String, String)> = Vec::new(); // (label, spilled_text)
    let mut remaining_excess = total.saturating_sub(budget);
    let mut trimmed_blocks: Vec<PromptTrimmedBlock> = Vec::new();

    for &idx in &trim_order {
        if remaining_excess == 0 {
            break;
        }

        let block_len = contents[idx].len();
        if block_len == 0 {
            continue;
        }

        let label = blocks[idx].label;

        if block_len <= remaining_excess {
            // Remove this block entirely.
            overflow_sections.push((label.to_string(), contents[idx].clone()));
            remaining_excess = remaining_excess.saturating_sub(block_len);
            contents[idx] = format!(
                "[{label} context ({block_len} chars) moved to overflow. NEXT: READ_MORE to see it.]"
            );
            trimmed_blocks.push(PromptTrimmedBlock {
                label: label.to_string(),
                original_chars: block_len,
                kept_chars: 0,
                removed_chars: block_len,
                fully_removed: true,
            });
        } else {
            // Partially trim this block.
            let keep_chars = block_len.saturating_sub(remaining_excess);
            let keep_at = find_paragraph_break(&contents[idx], keep_chars);
            let trimmed_portion = contents[idx][keep_at..].to_string();
            let trimmed_len = trimmed_portion.len();
            overflow_sections.push((label.to_string(), trimmed_portion));

            let mut kept: String = contents[idx][..keep_at].to_string();
            kept.push_str(&format!(
                "\n[...{trimmed_len} chars of {label} trimmed. NEXT: READ_MORE to see full context.]"
            ));
            remaining_excess = remaining_excess.saturating_sub(block_len.saturating_sub(keep_at));
            contents[idx] = kept;
            trimmed_blocks.push(PromptTrimmedBlock {
                label: label.to_string(),
                original_chars: block_len,
                kept_chars: keep_at,
                removed_chars: trimmed_len,
                fully_removed: false,
            });
        }
    }

    // Assemble in original block order.
    let assembled = contents
        .into_iter()
        .filter(|c| !c.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    // Write overflow to disk if anything was spilled.
    let overflow = if overflow_sections.is_empty() {
        None
    } else {
        let summary_parts: Vec<String> = overflow_sections
            .iter()
            .map(|(label, text)| format!("{label} ({} chars)", text.len()))
            .collect();
        let summary = summary_parts.join(", ");

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = overflow_dir.join(format!("context_overflow_{ts}.txt"));
        let _ = fs::create_dir_all(overflow_dir);

        let mut file_content = String::new();
        for (label, text) in &overflow_sections {
            file_content.push_str(&format!("=== [{label}] ===\n\n"));
            file_content.push_str(text);
            file_content.push_str("\n\n");
        }
        let _ = fs::write(&path, &file_content);

        Some(PromptOverflow {
            path,
            offset: 0,
            summary,
        })
    };

    let report = Some(PromptBudgetReport {
        budget,
        total_before: total,
        total_after: assembled.len(),
        trimmed_blocks,
    });

    (assembled, overflow, report)
}

/// Cap a string with overflow to disk. Returns (capped_content, optional overflow).
///
/// Used by individual callers (introspection, creation) for single large blocks.
pub fn cap_with_overflow(
    content: &str,
    label: &str,
    budget: usize,
    overflow_dir: &Path,
) -> (String, Option<PromptOverflow>) {
    if content.len() <= budget {
        return (content.to_string(), None);
    }

    let _ = fs::create_dir_all(overflow_dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = overflow_dir.join(format!("{label}_overflow_{ts}.txt"));
    let _ = fs::write(&path, content);

    let keep_at = find_paragraph_break(content, budget);
    let trimmed_len = content.len().saturating_sub(keep_at);
    let mut capped: String = content[..keep_at].to_string();
    capped.push_str(&format!(
        "\n\n[...{trimmed_len} more chars. NEXT: READ_MORE to continue reading.]"
    ));

    let overflow = PromptOverflow {
        path,
        offset: keep_at,
        summary: format!("{label} ({} chars total)", content.len()),
    };

    (capped, Some(overflow))
}

/// Clean up overflow files older than `max_age`.
pub fn cleanup_overflow_dir(dir: &Path, max_age: std::time::Duration) {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(std::time::UNIX_EPOCH);
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|t| t < cutoff)
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Find a paragraph or sentence break near `target_pos` in `text`.
///
/// Searches backward from `target_pos` for a blank line, period+space,
/// or newline. Falls back to `target_pos` if no natural break is found
/// within 200 chars.
fn find_paragraph_break(text: &str, target_pos: usize) -> usize {
    // Snap both endpoints to char boundaries to avoid panicking on multi-byte UTF-8.
    let mut target = target_pos.min(text.len());
    while target > 0 && !text.is_char_boundary(target) {
        target -= 1;
    }
    let mut search_start = target.saturating_sub(200);
    while search_start > 0 && !text.is_char_boundary(search_start) {
        search_start -= 1;
    }
    let slice = &text[search_start..target];

    // Prefer blank line.
    if let Some(pos) = slice.rfind("\n\n") {
        return search_start + pos + 2;
    }
    // Then period + space/newline.
    if let Some(pos) = slice.rfind(". ").or_else(|| slice.rfind(".\n")) {
        return search_start + pos + 2;
    }
    // Then any newline.
    if let Some(pos) = slice.rfind('\n') {
        return search_start + pos + 1;
    }
    // Fall back to exact position (snap to char boundary).
    let mut i = target;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_budget_returns_all_content() {
        let blocks = vec![
            PromptBlock {
                label: "a",
                content: "hello".into(),
                priority: 1,
            },
            PromptBlock {
                label: "b",
                content: "world".into(),
                priority: 2,
            },
        ];
        let dir = std::env::temp_dir().join("prompt_budget_test_under");
        let (assembled, overflow, report) = assemble_within_budget(blocks, 100, &dir);
        assert!(assembled.contains("hello"));
        assert!(assembled.contains("world"));
        assert!(overflow.is_none());
        assert!(report.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn over_budget_trims_lowest_priority_first() {
        let dir =
            std::env::temp_dir().join(format!("prompt_budget_test_trim_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let blocks = vec![
            PromptBlock {
                label: "high",
                content: "A".repeat(500),
                priority: 1,
            },
            PromptBlock {
                label: "medium",
                content: "B".repeat(500),
                priority: 3,
            },
            PromptBlock {
                label: "low",
                content: "C".repeat(500),
                priority: 5,
            },
        ];
        // Budget 800: total 1500, excess 700. "low" (priority 5) trimmed first.
        let (assembled, overflow, report) = assemble_within_budget(blocks, 800, &dir);

        // High-priority content should be fully preserved.
        assert!(assembled.contains(&"A".repeat(500)));
        // Low-priority should be trimmed with a notice.
        assert!(assembled.contains("READ_MORE"));
        // Overflow should exist.
        let of = overflow.expect("overflow should exist");
        assert!(of.path.exists());
        assert!(of.summary.contains("low"));
        let report = report.expect("budget report should exist");
        assert!(
            report
                .trimmed_blocks
                .iter()
                .any(|block| block.label == "low")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cap_with_overflow_preserves_short_content() {
        let dir = std::env::temp_dir().join("prompt_budget_test_cap");
        let (capped, overflow) = cap_with_overflow("short text", "test", 1000, &dir);
        assert_eq!(capped, "short text");
        assert!(overflow.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cap_with_overflow_spills_long_content() {
        let dir =
            std::env::temp_dir().join(format!("prompt_budget_test_spill_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Content must be substantially over budget so the notice doesn't
        // make the capped version longer than the original.
        let long_text = "First paragraph with details.\n\n\
            Second paragraph with more content.\n\n\
            Third paragraph explains the theory in depth.\n\n\
            Fourth paragraph has the conclusion and final thoughts about the research.";
        let (capped, overflow) = cap_with_overflow(long_text, "source", 60, &dir);

        assert!(capped.contains("READ_MORE"));
        let of = overflow.expect("overflow should exist");
        assert!(of.path.exists());
        assert!(of.offset > 0);
        // The full file on disk should contain everything.
        let disk_content = std::fs::read_to_string(&of.path).unwrap();
        assert_eq!(disk_content, long_text);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
