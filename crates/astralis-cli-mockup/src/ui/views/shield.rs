//! Shield view - Prioritized approval queue with bulk operations.

use crate::ui::state::{App, RiskLevel};
use crate::ui::theme::Theme;
use crate::ui::widgets::render_threat_indicator;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

pub(crate) fn render_shield(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // Layout: header + queue + detail panel (optional) + action bar
    let has_detail = app.shield_detail_expanded && !app.shield_approvals.is_empty();
    let has_selection = !app.shield_selected_items.is_empty();

    let mut constraints = vec![Constraint::Length(1)]; // header
    if has_detail {
        constraints.push(Constraint::Min(3)); // queue
        constraints.push(Constraint::Length(4)); // detail panel
    } else {
        constraints.push(Constraint::Min(3)); // queue
    }
    if has_selection {
        constraints.push(Constraint::Length(1)); // action bar
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // Header with threat level
    render_header(frame, chunks[0], app, theme);

    // Approval queue
    render_queue(frame, chunks[1], app, theme);

    // Detail panel
    let mut next_chunk = 2;
    if has_detail && chunks.len() > next_chunk {
        render_detail_panel(frame, chunks[next_chunk], app, theme);
        next_chunk += 1;
    }

    // Action bar
    if has_selection && chunks.len() > next_chunk {
        render_shield_action_bar(frame, chunks[next_chunk], app, theme);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let threat_line = render_threat_indicator(app.threat_level, theme);
    let pending_count = app.shield_approvals.len();

    let mut spans = vec![Span::styled(
        "  SHIELD ",
        Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
    )];
    spans.extend(threat_line.spans);
    spans.push(Span::styled(
        format!("  {pending_count} pending"),
        Style::default().fg(theme.muted),
    ));
    spans.push(Span::styled(
        "  Sort: [Risk]↓",
        Style::default().fg(theme.muted),
    ));

    let header = Paragraph::new(Line::from(spans));
    frame.render_widget(header, area);
}

#[allow(clippy::too_many_lines)]
fn render_queue(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    if app.shield_approvals.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No pending approvals. All clear.",
            Style::default().fg(theme.muted),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Active capabilities: ",
            Style::default().fg(theme.muted),
        )));
        for cap in &app.active_capabilities {
            let scope_color = match cap.scope.as_str() {
                "session" => theme.cap_session,
                "persistent" => theme.cap_persistent,
                _ => theme.muted,
            };
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    &cap.resource,
                    Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" (", Style::default().fg(theme.muted)),
                Span::styled(&cap.scope, Style::default().fg(scope_color)),
                Span::styled(
                    format!(", {} uses", cap.use_count),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(")", Style::default().fg(theme.muted)),
            ]));
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, area);
        return;
    }

    // Sort approvals by risk (High > Medium > Low)
    let mut sorted: Vec<(usize, &crate::ui::state::ApprovalSnapshot)> =
        app.shield_approvals.iter().enumerate().collect();
    sorted.sort_by(|a, b| {
        let risk_ord = |r: &RiskLevel| -> u8 {
            match r {
                RiskLevel::High => 0,
                RiskLevel::Medium => 1,
                RiskLevel::Low => 2,
            }
        };
        risk_ord(&a.1.risk_level).cmp(&risk_ord(&b.1.risk_level))
    });

    for (display_idx, (orig_idx, approval)) in sorted.iter().enumerate() {
        let is_focused = display_idx == app.shield_selected;
        let is_selected = app.shield_selected_items.contains(orig_idx);

        let risk_color = match approval.risk_level {
            RiskLevel::Low => theme.success,
            RiskLevel::Medium => theme.warning,
            RiskLevel::High => theme.error,
        };

        let focus_marker = if is_focused { "▸" } else { " " };
        let select_marker = if is_selected { "●" } else { "○" };

        let risk_label = match approval.risk_level {
            RiskLevel::Low => "LOW ",
            RiskLevel::Medium => "MED ",
            RiskLevel::High => "HIGH",
        };

        let elapsed = approval.timestamp.elapsed();
        let time_str = if elapsed.as_secs() >= 60 {
            format!("{}m ago", elapsed.as_secs() / 60)
        } else {
            format!("{}s ago", elapsed.as_secs())
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(" {focus_marker} "),
                Style::default().fg(if is_focused { theme.tool } else { theme.muted }),
            ),
            Span::styled(
                format!("{select_marker} "),
                Style::default().fg(if is_selected { theme.tool } else { theme.muted }),
            ),
            Span::styled(
                format!("{risk_label}  "),
                Style::default().fg(risk_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<12}", approval.agent_name),
                Style::default().fg(theme.tool),
            ),
            Span::styled(&approval.tool_name, Style::default().fg(theme.assistant)),
            Span::styled(format!("  {time_str:>8}"), Style::default().fg(theme.muted)),
        ]));

        // Show action hints on focused item
        if is_focused {
            lines.push(Line::from(vec![
                Span::styled("      ", Style::default()),
                Span::styled("[y]", Style::default().fg(theme.success)),
                Span::styled(" once  ", Style::default().fg(theme.muted)),
                Span::styled("[s]", Style::default().fg(theme.tool)),
                Span::styled(" session  ", Style::default().fg(theme.muted)),
                Span::styled("[a]", Style::default().fg(theme.success)),
                Span::styled(" always  ", Style::default().fg(theme.muted)),
                Span::styled("[n]", Style::default().fg(theme.error)),
                Span::styled(" deny  ", Style::default().fg(theme.muted)),
                Span::styled("[Enter]", Style::default().fg(theme.tool)),
                Span::styled(" detail", Style::default().fg(theme.muted)),
            ]));
        }
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}

fn render_detail_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.shield_approvals.is_empty() || app.shield_selected >= app.shield_approvals.len() {
        return;
    }

    let approval = &app.shield_approvals[app.shield_selected.min(app.shield_approvals.len() - 1)];

    let lines = vec![
        Line::from(Span::styled(
            format!("  ▾ {} — {}", approval.agent_name, approval.tool_name),
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("    Context: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("\"{}\"", approval.description),
                Style::default().fg(theme.assistant),
            ),
        ]),
        Line::from(vec![
            Span::styled("    Cap requested: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("mcp://{}  scope: once", approval.tool_name),
                Style::default().fg(theme.tool),
            ),
        ]),
    ];

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}

fn render_shield_action_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let count = app.shield_selected_items.len();
    let spans = vec![
        Span::styled(
            format!("  {count} selected: "),
            Style::default().fg(theme.tool),
        ),
        Span::styled("[Y]", Style::default().fg(theme.success)),
        Span::styled(" approve all  ", Style::default().fg(theme.muted)),
        Span::styled("[N]", Style::default().fg(theme.error)),
        Span::styled(" deny all", Style::default().fg(theme.muted)),
    ];

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}
