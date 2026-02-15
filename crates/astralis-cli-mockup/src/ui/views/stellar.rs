//! Stellar/Atlas view - File explorer.

use crate::ui::state::{App, FileEntryStatus};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub(crate) fn render_stellar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            "  Atlas ",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        Span::styled("— File Explorer", Style::default().fg(theme.muted)),
    ]));
    lines.push(Line::from(""));

    if app.files.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No files tracked yet.",
            Style::default().fg(theme.muted),
        )));
    } else {
        // Build file tree with connectors
        let total = app.files.len();
        for (i, entry) in app.files.iter().enumerate() {
            // Safety: total > 0 (we're iterating), and i + 1 checked before indexing
            #[allow(clippy::arithmetic_side_effects)]
            let is_last = i == total - 1 || (i + 1 < total && app.files[i + 1].depth < entry.depth);
            let connector = if is_last { "└── " } else { "├── " };
            let indent = "│   ".repeat(entry.depth.saturating_sub(1));
            let prefix = if entry.depth == 0 {
                if is_last { "└── " } else { "├── " }
            } else {
                connector
            };

            // Status indicator
            let (status_char, status_color) = match entry.status {
                FileEntryStatus::Unchanged => (" ", theme.muted),
                FileEntryStatus::Modified => ("M", theme.file_modified),
                FileEntryStatus::Added => ("A", theme.file_added),
                FileEntryStatus::Deleted => ("D", theme.file_deleted),
                FileEntryStatus::Editing => ("E", theme.thinking),
            };

            let name_style = if entry.is_dir {
                Style::default().fg(theme.tool).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.assistant)
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {indent}{prefix}"),
                    Style::default().fg(theme.border),
                ),
                Span::styled(&entry.path, name_style),
                Span::styled(
                    format!("  {status_char}"),
                    Style::default().fg(status_color),
                ),
            ]));
        }

        // Summary
        let modified = app
            .files
            .iter()
            .filter(|f| f.status == FileEntryStatus::Modified)
            .count();
        let added = app
            .files
            .iter()
            .filter(|f| f.status == FileEntryStatus::Added)
            .count();
        let deleted = app
            .files
            .iter()
            .filter(|f| f.status == FileEntryStatus::Deleted)
            .count();
        lines.push(Line::from(""));
        let mut summary_spans = vec![Span::styled("  ", Style::default())];
        if modified > 0 {
            summary_spans.push(Span::styled(
                format!("{modified}M"),
                Style::default().fg(theme.file_modified),
            ));
            summary_spans.push(Span::raw(" "));
        }
        if added > 0 {
            summary_spans.push(Span::styled(
                format!("{added}A"),
                Style::default().fg(theme.file_added),
            ));
            summary_spans.push(Span::raw(" "));
        }
        if deleted > 0 {
            summary_spans.push(Span::styled(
                format!("{deleted}D"),
                Style::default().fg(theme.file_deleted),
            ));
            summary_spans.push(Span::raw(" "));
        }
        if modified == 0 && added == 0 && deleted == 0 {
            summary_spans.push(Span::styled("No changes", Style::default().fg(theme.muted)));
        }
        lines.push(Line::from(summary_spans));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}
