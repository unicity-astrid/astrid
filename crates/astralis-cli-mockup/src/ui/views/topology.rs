//! Topology view - Agent hierarchy tree.

use crate::ui::state::{AgentStatus, App, SubAgentNode};
use crate::ui::theme::Theme;
use crate::ui::widgets::render_tree_node;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

#[allow(clippy::too_many_lines)]
pub(crate) fn render_topology(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    // Header with pool stats
    let active = app
        .subagent_tree
        .iter()
        .filter(|n| matches!(n.status, crate::ui::state::SubAgentStatus::Running))
        .count();
    let total = app.subagent_tree.len();

    lines.push(Line::from(vec![
        Span::styled(
            "  TOPOLOGY ",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  Pool: {active}/{total} active"),
            Style::default().fg(theme.muted),
        ),
    ]));
    lines.push(Line::from(""));

    if app.agents.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No agents running.",
            Style::default().fg(theme.muted),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  This view shows agent-to-sub-agent delegation chains.",
            Style::default().fg(theme.assistant),
        )));
    } else {
        // Render each agent and its sub-agents as a tree
        for agent in &app.agents {
            let status_color = match agent.status {
                AgentStatus::Ready => theme.agent_ready,
                AgentStatus::Busy => theme.agent_busy,
                AgentStatus::Error => theme.agent_error,
                AgentStatus::Paused => theme.agent_paused,
                AgentStatus::Starting => theme.thinking,
            };

            let status_label = match agent.status {
                AgentStatus::Ready => "[READY]",
                AgentStatus::Busy => "[BUSY]",
                AgentStatus::Error => "[ERROR]",
                AgentStatus::Paused => "[PAUSED]",
                AgentStatus::Starting => "[STARTING]",
            };

            let uptime = agent.last_activity.elapsed();
            let uptime_str = if uptime.as_secs() == 0 {
                "idle".to_string()
            } else if uptime.as_secs() >= 60 {
                format!("{}m {:02}s", uptime.as_secs() / 60, uptime.as_secs() % 60)
            } else {
                format!("{}s", uptime.as_secs())
            };

            // Agent header line with connecting dashes
            let name_len = agent.name.len() + status_label.len() + 4;
            let dash_count = (area.width as usize).saturating_sub(name_len + uptime_str.len() + 6);
            let dashes = "─".repeat(dash_count);

            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", agent.name),
                    Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    status_label,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {dashes} "), Style::default().fg(theme.border)),
                Span::styled(uptime_str, Style::default().fg(theme.muted)),
            ]));

            // Find sub-agents for this agent
            let children: Vec<&SubAgentNode> = app
                .subagent_tree
                .iter()
                .filter(|n| n.parent_agent == agent.name && n.parent_subagent.is_none())
                .collect();

            if children.is_empty() && agent.status == AgentStatus::Ready {
                // No sub-agents, nothing to show
            } else {
                for (i, child) in children.iter().enumerate() {
                    let is_last = i == children.len() - 1;
                    lines.push(render_tree_node(child, is_last, theme));

                    // Find grandchildren
                    let grandchildren: Vec<&SubAgentNode> = app
                        .subagent_tree
                        .iter()
                        .filter(|n| n.parent_subagent.as_deref() == Some(&child.id))
                        .collect();

                    for (j, grandchild) in grandchildren.iter().enumerate() {
                        let is_last_gc = j == grandchildren.len() - 1;
                        lines.push(render_tree_node(grandchild, is_last_gc, theme));
                    }
                }
            }

            lines.push(Line::from(""));
        }

        // Footer stats
        let completed = app
            .subagent_tree
            .iter()
            .filter(|n| matches!(n.status, crate::ui::state::SubAgentStatus::Completed))
            .count();
        let failed = app
            .subagent_tree
            .iter()
            .filter(|n| {
                matches!(
                    n.status,
                    crate::ui::state::SubAgentStatus::Failed
                        | crate::ui::state::SubAgentStatus::TimedOut
                )
            })
            .count();
        let max_depth = app.subagent_tree.iter().map(|n| n.depth).max().unwrap_or(0);

        lines.push(Line::from(vec![
            Span::styled(
                format!("  Active: {active}/{total}"),
                Style::default().fg(theme.assistant),
            ),
            Span::styled(
                format!("  Max Depth: {max_depth}"),
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                format!("  Completed: {completed}"),
                Style::default().fg(theme.success),
            ),
            Span::styled(
                format!("  Failed: {failed}"),
                Style::default().fg(if failed > 0 { theme.error } else { theme.muted }),
            ),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}
