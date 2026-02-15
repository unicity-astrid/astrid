//! Command view - Sortable agent table with multi-select and bulk operations.

use crate::ui::state::{AgentStatus, App, CommandSort};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

pub(crate) fn render_command(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.agents.is_empty() {
        render_empty_command(frame, area, theme);
        return;
    }

    // Layout: header + table + action bar
    let has_selection = !app.command_selected.is_empty();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_selection {
            vec![
                Constraint::Length(1), // Header
                Constraint::Min(3),    // Table
                Constraint::Length(1), // Action bar
            ]
        } else {
            vec![
                Constraint::Length(1), // Header
                Constraint::Min(3),    // Table
            ]
        })
        .split(area);

    // Header
    render_header(frame, chunks[0], app, theme);

    // Table
    render_table(frame, chunks[1], app, theme);

    // Action bar (only when items selected)
    if has_selection && chunks.len() > 2 {
        render_action_bar(frame, chunks[2], app, theme);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let sort_arrow = app.command_sort_dir.arrow();

    let mut spans = vec![
        Span::styled(
            "  COMMAND ",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} agents ", app.agents.len()),
            Style::default().fg(theme.muted),
        ),
        Span::styled(" Sort: ", Style::default().fg(theme.muted)),
    ];

    // Show all sort columns, highlight active
    let all_sorts = [
        CommandSort::Name,
        CommandSort::Status,
        CommandSort::Activity,
        CommandSort::Budget,
        CommandSort::SubAgents,
        CommandSort::Context,
    ];

    for s in &all_sorts {
        let is_active = app.command_sort == *s;
        if is_active {
            spans.push(Span::styled(
                format!("[{}]{sort_arrow}", s.label()),
                Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", s.label()),
                Style::default().fg(theme.muted),
            ));
        }
    }

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}

#[allow(clippy::too_many_lines)]
fn render_table(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    // Table header
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(format!("{:<3}", ""), Style::default().fg(theme.muted)),
        Span::styled(
            format!("{:<12}", "Agent"),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<8}", "Status"),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<12}", "Activity"),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<8}", "Budget"),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<6}", "Subs"),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Ctx%",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Separator
    let sep_width = area.width.saturating_sub(2) as usize;
    lines.push(Line::from(Span::styled(
        format!("  {}", "─".repeat(sep_width.min(70))),
        Style::default().fg(theme.border),
    )));

    // Agent rows (sorted)
    let mut sorted_indices: Vec<usize> = (0..app.agents.len()).collect();
    sorted_indices.sort_by(|&a, &b| {
        let agent_a = &app.agents[a];
        let agent_b = &app.agents[b];
        let ordering = match app.command_sort {
            CommandSort::Name => agent_a.name.cmp(&agent_b.name),
            CommandSort::Status => {
                format!("{:?}", agent_a.status).cmp(&format!("{:?}", agent_b.status))
            },
            CommandSort::Activity => agent_a.current_activity.cmp(&agent_b.current_activity),
            CommandSort::Budget => agent_a
                .budget_spent
                .partial_cmp(&agent_b.budget_spent)
                .unwrap_or(std::cmp::Ordering::Equal),
            CommandSort::SubAgents => agent_a.active_subagents.cmp(&agent_b.active_subagents),
            CommandSort::Context => agent_a
                .context_usage
                .partial_cmp(&agent_b.context_usage)
                .unwrap_or(std::cmp::Ordering::Equal),
        };
        match app.command_sort_dir {
            crate::ui::state::SortDirection::Ascending => ordering,
            crate::ui::state::SortDirection::Descending => ordering.reverse(),
        }
    });

    for (row_idx, &agent_idx) in sorted_indices.iter().enumerate() {
        let agent = &app.agents[agent_idx];
        let is_focused = row_idx == app.selected_agent;
        let is_selected = app.command_selected.contains(&agent_idx);

        let checkbox = if is_selected { "[x]" } else { "[ ]" };
        let row_style = if is_focused {
            Style::default().fg(theme.user)
        } else {
            Style::default().fg(theme.assistant)
        };

        let (status_str, status_color) = match agent.status {
            AgentStatus::Ready => ("IDLE", theme.muted),
            AgentStatus::Busy => ("BUSY", theme.tool),
            AgentStatus::Paused => ("PAUSE", theme.warning),
            AgentStatus::Error => (" ERR", theme.error),
            AgentStatus::Starting => ("START", theme.thinking),
        };

        let activity = agent.current_activity.as_deref().unwrap_or("—");
        let activity_short: String = activity.chars().take(10).collect();

        let budget_str = format!("${:.2}", agent.budget_spent);

        // Sub-agent info
        let subs_str = if agent.active_subagents > 0 {
            format!("{}", agent.active_subagents)
        } else {
            "0".to_string()
        };

        // Context bar
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ctx_pct = (agent.context_usage * 100.0) as u8;
        let bar_width = 8usize;
        let filled = (usize::from(ctx_pct) * bar_width) / 100;
        let empty = bar_width.saturating_sub(filled);
        let bar_color = if ctx_pct > 80 {
            theme.error
        } else if ctx_pct > 60 {
            theme.warning
        } else {
            theme.success
        };

        let focus_prefix = if is_focused { "▸ " } else { "  " };

        lines.push(Line::from(vec![
            Span::styled(
                focus_prefix,
                Style::default().fg(if is_focused { theme.tool } else { theme.muted }),
            ),
            Span::styled(
                format!("{checkbox} "),
                Style::default().fg(if is_selected { theme.tool } else { theme.muted }),
            ),
            Span::styled(format!("{:<12}", agent.name), row_style),
            Span::styled(
                format!("{status_str:<8}"),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{activity_short:<12}"),
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                format!("{budget_str:<8}"),
                Style::default().fg(theme.assistant),
            ),
            Span::styled(format!("{subs_str:<6}"), Style::default().fg(theme.muted)),
            Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
            Span::styled("░".repeat(empty), Style::default().fg(theme.border)),
            Span::styled(format!(" {ctx_pct:>2}"), Style::default().fg(bar_color)),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}

fn render_action_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let count = app.command_selected.len();
    let spans = vec![
        Span::styled(
            format!("  {count} selected: "),
            Style::default().fg(theme.tool),
        ),
        Span::styled("[r]", Style::default().fg(theme.success)),
        Span::styled("estart ", Style::default().fg(theme.muted)),
        Span::styled("[p]", Style::default().fg(theme.warning)),
        Span::styled("ause ", Style::default().fg(theme.muted)),
        Span::styled("[k]", Style::default().fg(theme.error)),
        Span::styled("ill ", Style::default().fg(theme.muted)),
        Span::styled("[b]", Style::default().fg(theme.tool)),
        Span::styled("udget+$10  ", Style::default().fg(theme.muted)),
        Span::styled("[Enter]", Style::default().fg(theme.tool)),
        Span::styled("→Nexus", Style::default().fg(theme.muted)),
    ];

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}

fn render_empty_command(frame: &mut Frame, area: Rect, theme: &Theme) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  COMMAND",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  No agents running.",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            "  Start a session to see agents here.",
            Style::default().fg(theme.muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  This view shows all agents in a sortable table:",
            Style::default().fg(theme.assistant),
        )),
        Line::from(Span::styled(
            "  - Status, current activity, budget per agent",
            Style::default().fg(theme.assistant),
        )),
        Line::from(Span::styled(
            "  - Sub-agent counts and context usage",
            Style::default().fg(theme.assistant),
        )),
        Line::from(Span::styled(
            "  - Multi-select with Space, bulk operations",
            Style::default().fg(theme.assistant),
        )),
        Line::from(Span::styled(
            "  - [s] sort  [r] reverse  [Space] select  [Enter] focus",
            Style::default().fg(theme.assistant),
        )),
    ];

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}
