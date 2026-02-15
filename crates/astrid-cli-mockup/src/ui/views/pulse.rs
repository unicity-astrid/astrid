//! Pulse view - Health, Budget, Performance dashboard (2x2 grid).

use crate::ui::state::{App, HealthStatus, OverallHealth};
use crate::ui::theme::Theme;
use crate::ui::widgets::{render_budget_bar, render_gauge_bar};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

pub(crate) fn render_pulse(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // Header
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);

    let uptime_str = format_uptime(app.gateway_uptime);
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "  PULSE ",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  Uptime: {uptime_str}"),
            Style::default().fg(theme.muted),
        ),
    ]));
    frame.render_widget(header, chunks[0]);

    // 2x2 grid
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    render_health_panel(frame, top_cols[0], app, theme);
    render_budget_panel(frame, top_cols[1], app, theme);
    render_tokens_panel(frame, bot_cols[0], app, theme);
    render_performance_panel(frame, bot_cols[1], app, theme);
}

fn render_health_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::default()
        .title(" Health Status ")
        .title_style(Style::default().fg(theme.muted))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    if app.health.checks.is_empty() {
        lines.push(Line::from(Span::styled(
            " No health data",
            Style::default().fg(theme.muted),
        )));
    } else {
        for check in &app.health.checks {
            let (status_str, color) = match check.status {
                HealthStatus::Ok => ("[OK]", theme.success),
                HealthStatus::Degraded => ("[DEGRADED]", theme.warning),
                HealthStatus::Down => ("[DOWN]", theme.error),
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:<16}", check.component),
                    Style::default().fg(theme.assistant),
                ),
                Span::styled(
                    format!("{status_str:<12}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:.0}ms", check.latency_ms),
                    Style::default().fg(theme.muted),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));
    let overall_str = match app.health.overall {
        OverallHealth::Healthy => ("Overall: HEALTHY", theme.success),
        OverallHealth::Degraded => ("Overall: DEGRADED", theme.warning),
        OverallHealth::Unhealthy => ("Overall: UNHEALTHY", theme.error),
    };
    lines.push(Line::from(Span::styled(
        format!(" {}", overall_str.0),
        Style::default()
            .fg(overall_str.1)
            .add_modifier(Modifier::BOLD),
    )));

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn render_budget_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::default()
        .title(" Budget Overview ")
        .title_style(Style::default().fg(theme.muted))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        " Session:",
        Style::default().fg(theme.muted),
    )));

    let bar_width = inner.width.saturating_sub(4) as usize;
    lines.push(render_budget_bar(
        app.budget.total_spent,
        app.budget.session_limit,
        bar_width.min(16),
        theme,
    ));

    // Per-agent breakdown
    if !app.budget.per_agent.is_empty() {
        lines.push(Line::from(""));
        for (agent, spent) in &app.budget.per_agent {
            lines.push(Line::from(vec![
                Span::styled(format!(" {agent}: "), Style::default().fg(theme.tool)),
                Span::styled(format!("${spent:.2}"), Style::default().fg(theme.assistant)),
            ]));
        }
    }

    // Burn rate
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" Burn Rate: ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("${:.2}/hr", app.budget.burn_rate_per_hour),
            Style::default().fg(theme.assistant),
        ),
    ]));

    if app.budget.burn_rate_per_hour > 0.0 {
        let remaining = app.budget.session_limit - app.budget.total_spent;
        let est_hours = remaining / app.budget.burn_rate_per_hour;
        lines.push(Line::from(vec![
            Span::styled(" Est: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{est_hours:.0}h remaining"),
                Style::default().fg(theme.assistant),
            ),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn render_tokens_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::default()
        .title(" Token Usage ")
        .title_style(Style::default().fg(theme.muted))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(" Input:  ", Style::default().fg(theme.muted)),
        Span::styled(
            format_tokens(app.budget.input_tokens),
            Style::default().fg(theme.assistant),
        ),
        Span::styled("  Output: ", Style::default().fg(theme.muted)),
        Span::styled(
            format_tokens(app.budget.output_tokens),
            Style::default().fg(theme.assistant),
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Context per agent:",
        Style::default().fg(theme.muted),
    )));

    let bar_width = inner.width.saturating_sub(12) as usize;
    for agent in &app.agents {
        lines.push(render_gauge_bar(
            &format!(" {}", agent.name),
            agent.context_usage,
            bar_width.min(10),
            theme,
        ));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn render_performance_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::default()
        .title(" Performance ")
        .title_style(Style::default().fg(theme.muted))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let perf = &app.performance;
    let lines = vec![
        Line::from(vec![
            Span::styled(" Avg Tool Latency:   ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{:.0}ms", perf.avg_tool_latency_ms),
                Style::default().fg(theme.assistant),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Avg LLM Latency:    ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{:.1}s", perf.avg_llm_latency_ms / 1000.0),
                Style::default().fg(theme.assistant),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Avg Approval Wait:  ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{:.1}s", perf.avg_approval_wait_ms / 1000.0),
                Style::default().fg(theme.assistant),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Tool Calls/min:     ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{:.1}", perf.tool_calls_per_min),
                Style::default().fg(theme.assistant),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Events/min:        ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{:.1}", perf.events_per_min),
                Style::default().fg(theme.assistant),
            ),
        ]),
    ];

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn format_uptime(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{hours}h {mins:02}m {s:02}s")
}

#[allow(clippy::cast_precision_loss)]
fn format_tokens(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        format!("{tokens}")
    }
}
