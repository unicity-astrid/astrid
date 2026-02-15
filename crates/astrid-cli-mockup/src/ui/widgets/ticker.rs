//! Scrolling event ticker widget for Command view.

use crate::ui::state::{EventCategory, EventRecord};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Render the event ticker showing recent events.
pub(crate) fn render_ticker(frame: &mut Frame, area: Rect, events: &[EventRecord], theme: &Theme) {
    let block = Block::default()
        .title(" Event Ticker ")
        .title_style(Style::default().fg(theme.muted))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if events.is_empty() {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "  No events yet...",
            Style::default().fg(theme.muted),
        )));
        frame.render_widget(placeholder, inner);
        return;
    }

    let visible = inner.height as usize;
    let start = events.len().saturating_sub(visible);
    let session_start = events
        .first()
        .map_or_else(std::time::Instant::now, |e| e.timestamp);

    let lines: Vec<Line> = events[start..]
        .iter()
        .map(|event| {
            let elapsed = event.timestamp.duration_since(session_start);
            let secs = elapsed.as_secs();
            let time_str = if secs >= 60 {
                format!("{:>2}:{:02}", secs / 60, secs % 60)
            } else {
                format!("{secs:>5}")
            };

            let event_color = category_color(event.category, theme);

            Line::from(vec![
                Span::styled(format!("  {time_str}  "), Style::default().fg(theme.muted)),
                Span::styled(
                    format!("{:<8}", event.agent_name),
                    Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<20}", event.event_type),
                    Style::default().fg(event_color),
                ),
                Span::styled(&event.detail, Style::default().fg(theme.assistant)),
            ])
        })
        .collect();

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn category_color(cat: EventCategory, theme: &Theme) -> ratatui::style::Color {
    match cat {
        EventCategory::Session => theme.success,
        EventCategory::Tool => theme.tool,
        EventCategory::Approval => theme.warning,
        EventCategory::Error => theme.error,
        EventCategory::Security => theme.thinking,
        EventCategory::Llm => theme.assistant,
        EventCategory::Runtime => theme.muted,
    }
}
