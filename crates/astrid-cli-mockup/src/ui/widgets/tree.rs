//! Tree rendering widget for Topology view.

use crate::ui::state::{SubAgentNode, SubAgentStatus};
use crate::ui::theme::Theme;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// Render a sub-agent tree node as a Line.
pub(crate) fn render_tree_node<'a>(
    node: &SubAgentNode,
    is_last_sibling: bool,
    _theme: &Theme,
) -> Line<'a> {
    let theme = crate::ui::Theme::default(); // Use default to get owned colors

    let indent = if node.depth > 0 {
        let pipe = "│   ".repeat(node.depth.saturating_sub(1));
        let connector = if is_last_sibling {
            "└── "
        } else {
            "├── "
        };
        format!("  {pipe}{connector}")
    } else {
        "  ".to_string()
    };

    let (status_label, status_color) = match node.status {
        SubAgentStatus::Running => ("[RUNNING]", theme.agent_busy),
        SubAgentStatus::Completed => ("[COMPLETED]", theme.agent_ready),
        SubAgentStatus::Failed => ("[FAILED]", theme.agent_error),
        SubAgentStatus::TimedOut => ("[TIMED_OUT]", theme.agent_error),
        SubAgentStatus::Cancelled => ("[CANCELLED]", theme.agent_paused),
    };

    let duration_str = if let Some(d) = node.duration {
        let secs = d.as_secs();
        if secs >= 60 {
            format!("{}m {:02}s", secs / 60, secs % 60)
        } else {
            format!("{secs}s")
        }
    } else {
        let elapsed = node.started_at.elapsed();
        let secs = elapsed.as_secs();
        if secs >= 60 {
            format!("{}m {:02}s", secs / 60, secs % 60)
        } else {
            format!("{secs}s")
        }
    };

    let outcome = match node.status {
        SubAgentStatus::Completed => " OK",
        SubAgentStatus::Failed => " FAIL",
        SubAgentStatus::TimedOut => " TIMEOUT",
        _ => "",
    };

    Line::from(vec![
        Span::styled(indent, Style::default().fg(theme.border)),
        Span::styled(
            format!("{} ", node.id),
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            status_label,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" \"{}\"", node.task),
            Style::default().fg(theme.assistant),
        ),
        Span::styled(
            format!("  {duration_str}"),
            Style::default().fg(theme.muted),
        ),
        Span::styled(outcome, Style::default().fg(status_color)),
    ])
}
