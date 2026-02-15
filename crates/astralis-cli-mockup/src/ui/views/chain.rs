//! Chain view - Live audit trail with integrity verification.

use crate::ui::state::{App, AuditFilter, AuditOutcome};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub(crate) fn render_chain(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // Header + filter bar + entries
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header with integrity status
            Constraint::Length(1), // Filter bar
            Constraint::Min(3),    // Entries
        ])
        .split(area);

    // Header
    render_chain_header(frame, chunks[0], app, theme);

    // Filter bar
    render_chain_filters(frame, chunks[1], app, theme);

    // Entries
    render_chain_entries(frame, chunks[2], app, theme);
}

fn render_chain_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let integrity = &app.chain_integrity;
    let (integrity_label, integrity_color) = if integrity.verified {
        ("[VERIFIED]", theme.chain_verified)
    } else if let Some(break_at) = integrity.break_at {
        let label = format!("[BROKEN at #{break_at}]");
        // We need a static string for Span, so use owned
        let para = Paragraph::new(Line::from(vec![
            Span::styled(
                "  CHAIN ",
                Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Integrity: ", Style::default().fg(theme.muted)),
            Span::styled(
                label,
                Style::default()
                    .fg(theme.chain_broken)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {} entries  {} breaks",
                    integrity.total_entries,
                    i32::from(integrity.break_at.is_some())
                ),
                Style::default().fg(theme.muted),
            ),
        ]));
        frame.render_widget(para, area);
        return;
    } else {
        ("[UNVERIFIED]", theme.muted)
    };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "  CHAIN ",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Integrity: ", Style::default().fg(theme.muted)),
        Span::styled(
            integrity_label,
            Style::default()
                .fg(integrity_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} entries  0 breaks", integrity.total_entries),
            Style::default().fg(theme.muted),
        ),
    ]));
    frame.render_widget(header, area);
}

fn render_chain_filters(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let filters = [
        AuditFilter::All,
        AuditFilter::Security,
        AuditFilter::Tools,
        AuditFilter::Sessions,
        AuditFilter::Llm,
    ];

    let mut spans: Vec<Span> = vec![Span::styled("  Filter: ", Style::default().fg(theme.muted))];

    for f in &filters {
        let is_active = app.audit_filter == *f;
        if is_active {
            spans.push(Span::styled(
                format!("[{}]", f.label()),
                Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", f.label()),
                Style::default().fg(theme.muted),
            ));
        }
    }

    // Agent filter
    if let Some(ref agent) = app.audit_agent_filter {
        spans.push(Span::styled("  Agent: ", Style::default().fg(theme.muted)));
        spans.push(Span::styled(
            format!("[{agent}]"),
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            "  Agent: [all]",
            Style::default().fg(theme.muted),
        ));
    }

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}

#[allow(clippy::too_many_lines)]
fn render_chain_entries(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.audit_entries.is_empty() {
        let para = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No audit entries yet.",
                Style::default().fg(theme.muted),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  This view shows the cryptographic audit chain:",
                Style::default().fg(theme.assistant),
            )),
            Line::from(Span::styled(
                "  - Chain-linked, signed entries",
                Style::default().fg(theme.assistant),
            )),
            Line::from(Span::styled(
                "  - Integrity verification",
                Style::default().fg(theme.assistant),
            )),
            Line::from(Span::styled(
                "  - Filter by type or agent",
                Style::default().fg(theme.assistant),
            )),
        ]);
        frame.render_widget(para, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Filter entries
    let filtered: Vec<_> = app
        .audit_entries
        .iter()
        .filter(|e| matches_audit_filter(e, app.audit_filter))
        .filter(|e| {
            app.audit_agent_filter.is_none()
                || app.audit_agent_filter.as_deref() == Some(&e.agent_name)
        })
        .collect();

    // Apply scroll
    // Safety: division by nonzero literal 3
    #[allow(clippy::arithmetic_side_effects)]
    let visible_count = (area.height as usize) / 3; // ~3 lines per entry
    let start = filtered
        .len()
        .saturating_sub(visible_count.saturating_add(app.audit_scroll));
    let end = filtered.len().saturating_sub(app.audit_scroll);

    for entry in filtered.iter().skip(start).take(end.saturating_sub(start)) {
        let outcome_color = match entry.outcome {
            AuditOutcome::Success => theme.success,
            AuditOutcome::Failure | AuditOutcome::Violation => theme.error,
            AuditOutcome::Denied => theme.warning,
        };

        let action_color =
            if entry.action.contains("Security") || entry.action.contains("Violation") {
                theme.error
            } else if entry.action.contains("Approval") {
                theme.warning
            } else if entry.action.contains("Tool") || entry.action.contains("Mcp") {
                theme.tool
            } else if entry.action.contains("Session") {
                theme.success
            } else {
                theme.muted
            };

        // Entry header line
        let session_start = app
            .audit_entries
            .front()
            .map_or_else(std::time::Instant::now, |e| e.timestamp);
        let elapsed = entry.timestamp.duration_since(session_start);
        let secs = elapsed.as_secs();
        let time_str = if secs >= 60 {
            format!("{:>2}:{:02}", secs / 60, secs % 60)
        } else {
            format!("{secs:>5}")
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(" #{:<4}", entry.id),
                Style::default().fg(theme.muted),
            ),
            Span::styled(format!("| {time_str} "), Style::default().fg(theme.muted)),
            Span::styled(
                format!("| {:<8}", entry.agent_name),
                Style::default().fg(theme.tool),
            ),
            Span::styled(
                format!("| {:<16}", entry.action),
                Style::default().fg(action_color),
            ),
            Span::styled(
                format!("| {}", entry.detail),
                Style::default().fg(theme.assistant),
            ),
        ]));

        // Auth + outcome line
        lines.push(Line::from(vec![
            Span::styled("      |          ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("| Auth: {:<10}", entry.auth_method),
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                format!("| {:?}", entry.outcome),
                Style::default().fg(outcome_color),
            ),
            Span::styled(
                format!("  Hash: {}...", &entry.hash[..entry.hash.len().min(8)]),
                Style::default().fg(theme.muted),
            ),
        ]));

        // Separator
        lines.push(Line::from(Span::styled(
            " ─────┼──────────┼──────────┼─────────────────────────────",
            Style::default().fg(theme.border),
        )));
    }

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

fn matches_audit_filter(entry: &crate::ui::state::AuditSnapshot, filter: AuditFilter) -> bool {
    match filter {
        AuditFilter::All => true,
        AuditFilter::Security => {
            entry.action.contains("Security")
                || entry.action.contains("Approval")
                || entry.action.contains("Violation")
                || entry.action.contains("Capability")
        },
        AuditFilter::Tools => entry.action.contains("Tool") || entry.action.contains("Mcp"),
        AuditFilter::Sessions => entry.action.contains("Session"),
        AuditFilter::Llm => entry.action.contains("Llm") || entry.action.contains("Request"),
    }
}
