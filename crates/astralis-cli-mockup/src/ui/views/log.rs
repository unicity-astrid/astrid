//! Log/Console view - Minimal mode with no sidebar.

use crate::ui::render::{markdown_to_spans, render_inline_tool};
use crate::ui::state::{App, MessageKind, MessageRole, UiState};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

#[allow(clippy::too_many_lines)]
pub(crate) fn render_log(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    // Minimal chrome: ⏺ bullets like Comms view, but with dashed separators
    for msg in &app.messages {
        // Handle inline tool results
        if let Some(MessageKind::ToolResult(idx)) = &msg.kind {
            render_inline_tool(&mut lines, app, *idx, theme);
        } else {
            match msg.role {
                MessageRole::User => {
                    for (i, line) in msg.content.lines().enumerate() {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled("> ", Style::default().fg(theme.tool)),
                                Span::styled(line, Style::default().fg(theme.user)),
                            ]));
                        } else {
                            lines.push(Line::from(Span::styled(
                                format!("  {line}"),
                                Style::default().fg(theme.user),
                            )));
                        }
                    }
                },
                MessageRole::Assistant => {
                    // Same ⏺/⎿ pattern as Comms view
                    let content_lines: Vec<&str> = msg.content.lines().collect();
                    for (i, line) in content_lines.iter().enumerate() {
                        if i == 0 {
                            let mut spans =
                                vec![Span::styled("⏺ ", Style::default().fg(Color::White))];
                            spans.extend(markdown_to_spans(line, theme));
                            lines.push(Line::from(spans));
                        } else {
                            let mut spans =
                                vec![Span::styled("  ⎿ ", Style::default().fg(theme.border))];
                            spans.extend(markdown_to_spans(line, theme));
                            lines.push(Line::from(spans));
                        }
                    }
                },
                MessageRole::System => {
                    let style = match &msg.kind {
                        Some(MessageKind::DiffHeader | MessageKind::DiffFooter) => {
                            Style::default().fg(theme.diff_context)
                        },
                        Some(MessageKind::DiffRemoved) => Style::default().fg(theme.diff_removed),
                        Some(MessageKind::DiffAdded) => Style::default().fg(theme.diff_added),
                        None => Style::default().fg(theme.muted),
                        Some(MessageKind::ToolResult(_)) => unreachable!(),
                    };
                    let prefix = if msg.kind.is_some() { "  ⎿  " } else { "" };
                    for line in msg.content.lines() {
                        lines.push(Line::from(Span::styled(format!("{prefix}{line}"), style)));
                    }
                },
            }
        } // close else for ToolResult check
        // Thin separator between message groups in log view
        if msg.spacing {
            lines.push(Line::from(Span::styled(
                "╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌",
                Style::default().fg(theme.border),
            )));
        }
    }

    // Thinking/streaming indicators (same as Nexus)
    if let UiState::Thinking { start_time, .. } = &app.state {
        let elapsed = start_time.elapsed();
        lines.push(Line::from(vec![
            Span::styled("... ", Style::default().fg(theme.thinking)),
            Span::styled(
                format!("{:.1}s", elapsed.as_secs_f32()),
                Style::default().fg(theme.muted),
            ),
        ]));
    }

    // Scroll to bottom
    let visible_height = area.height as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let effective_scroll = app.scroll_offset.min(max_scroll);
    let start_line = if total_lines > visible_height {
        max_scroll.saturating_sub(effective_scroll)
    } else {
        0
    };
    let end_line = start_line.saturating_add(visible_height).min(total_lines);
    // Safety: end_line >= start_line by construction (min of sum with total_lines)
    #[allow(clippy::arithmetic_side_effects)]
    let take_count = end_line - start_line;
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(start_line)
        .take(take_count)
        .collect();

    let paragraph = Paragraph::new(visible_lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}
