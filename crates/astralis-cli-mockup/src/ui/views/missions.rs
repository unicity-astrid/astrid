//! Missions view - Kanban task board with agent grouping.

use crate::ui::state::{App, TaskColumn};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem},
};

pub(crate) fn render_missions(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // Split into columns for the kanban board
    let columns = [
        (TaskColumn::Backlog, "Backlog", "○"),
        (TaskColumn::Active, "Active", "◐"),
        (TaskColumn::Review, "Review", "✧"),
        (TaskColumn::Complete, "Complete", "★"),
        (TaskColumn::Queued, "Queued", "◇"),
    ];

    // Calculate column widths (equal distribution)
    #[allow(clippy::cast_possible_truncation)]
    let col_count = columns.len() as u16;
    let constraints: Vec<Constraint> = (0..col_count)
        .map(|_| Constraint::Ratio(1, u32::from(col_count)))
        .collect();

    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, (column, title, icon)) in columns.iter().enumerate() {
        let tasks_in_column: Vec<&crate::ui::state::Task> =
            app.tasks.iter().filter(|t| &t.column == column).collect();

        let mut items: Vec<ListItem> = Vec::new();

        for task in tasks_in_column {
            let mut spans = vec![
                Span::styled(format!("{icon} "), Style::default().fg(theme.tool)),
                Span::styled(&task.title, Style::default().fg(theme.assistant)),
            ];

            // Show agent name if set
            if let Some(ref agent) = task.agent_name {
                spans.push(Span::styled(
                    format!(" [{agent}]"),
                    Style::default().fg(theme.muted),
                ));
            }

            items.push(ListItem::new(Line::from(spans)));
        }

        // If no tasks, show a placeholder
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "(empty)",
                Style::default().fg(theme.muted),
            ))));
        }

        let block = Block::default()
            .title(format!(" {title} "))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border));

        let list = List::new(items).block(block);
        frame.render_widget(list, col_chunks[i]);
    }
}
