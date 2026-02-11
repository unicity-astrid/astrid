//! Agent status card widget for Command view.

use crate::ui::state::{AgentSnapshot, AgentStatus};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

/// Render an agent card at the given area.
pub(crate) fn render_agent_card(
    frame: &mut Frame,
    area: Rect,
    agent: &AgentSnapshot,
    is_selected: bool,
    theme: &Theme,
) {
    let status_color = status_to_color(agent.status, theme);

    let border_style = if is_selected {
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(status_color)
    };

    let block = Block::default()
        .title(format!(" {} ", agent.name))
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Status + uptime
    let uptime = agent.last_activity.elapsed();
    let uptime_str = format_uptime(uptime);
    let status_label = match agent.status {
        AgentStatus::Ready => "[READY]",
        AgentStatus::Busy => "[BUSY]",
        AgentStatus::Error => "[ERROR]",
        AgentStatus::Paused => "[PAUSED]",
        AgentStatus::Starting => "[STARTING]",
    };
    lines.push(Line::from(vec![
        Span::styled(
            status_label,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {uptime_str}"), Style::default().fg(theme.muted)),
    ]));

    // Current activity
    if let Some(ref tool) = agent.current_tool {
        let spinner = theme
            .spinner
            .frame_at(agent.last_activity.elapsed().as_millis());
        lines.push(Line::from(vec![
            Span::styled(format!("{spinner} "), Style::default().fg(theme.tool)),
            Span::styled(tool.clone(), Style::default().fg(theme.tool)),
        ]));
    } else if let Some(ref activity) = agent.current_activity {
        lines.push(Line::from(Span::styled(
            activity.clone(),
            Style::default().fg(theme.assistant),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "Waiting...",
            Style::default().fg(theme.muted),
        )));
    }

    // Tokens + budget
    let tokens_str = format_tokens(agent.tokens_used);
    lines.push(Line::from(vec![
        Span::styled("tokens: ", Style::default().fg(theme.muted)),
        Span::styled(tokens_str, Style::default().fg(theme.assistant)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("budget: ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("${:.2}", agent.budget_spent),
            Style::default().fg(theme.assistant),
        ),
    ]));

    // Sub-agents
    lines.push(Line::from(vec![
        Span::styled("sub: ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("{} active", agent.active_subagents),
            Style::default().fg(theme.assistant),
        ),
    ]));

    // Pending approvals
    if agent.pending_approvals > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!("[!] {} pending", agent.pending_approvals),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )]));
    }

    // Error
    if let Some(ref err) = agent.last_error {
        lines.push(Line::from(Span::styled(
            format!("! {}", truncate(err, inner.width as usize - 2)),
            Style::default().fg(theme.error),
        )));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn status_to_color(status: AgentStatus, theme: &Theme) -> Color {
    match status {
        AgentStatus::Ready => theme.agent_ready,
        AgentStatus::Busy => theme.agent_busy,
        AgentStatus::Error => theme.agent_error,
        AgentStatus::Paused => theme.agent_paused,
        AgentStatus::Starting => theme.thinking,
    }
}

fn format_uptime(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

#[allow(clippy::cast_precision_loss)]
fn format_tokens(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        format!("{tokens}")
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max.min(s.len())]
    }
}
