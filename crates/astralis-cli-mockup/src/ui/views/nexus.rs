//! Nexus view - Unified observation plane.
//!
//! All events (conversation, tools, security, approvals, sub-agents, audit)
//! flow into a single chronological filtered stream. In single-agent mode,
//! defaults to Conversation filter for Claude Code simplicity.

use crate::ui::render::{markdown_to_spans, render_inline_tool, to_pascal_case};
use crate::ui::state::{
    App, AuditOutcome, MessageKind, MessageRole, NexusCategory, NexusEntry, RiskLevel,
    SubAgentStatus, ThreatLevel,
};
use crate::ui::theme::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

#[allow(clippy::too_many_lines)]
pub(crate) fn render_messages(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let multi_agent = app.agents.len() > 1;

    // Determine how many header rows we need
    let header_rows = if multi_agent { 2 } else { 0 };

    let (header_areas, content_area) = if header_rows > 0 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_rows), Constraint::Min(3)])
            .split(area);

        let header_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(chunks[0]);

        (Some((header_chunks[0], header_chunks[1])), chunks[1])
    } else {
        (None, area)
    };

    // Render header bars in multi-agent mode
    if let Some((filter_area, selector_area)) = header_areas {
        render_nexus_filter_bar(frame, filter_area, app, theme);
        render_agent_selector(frame, selector_area, app, theme);
    }

    // Build lines from the nexus stream
    let mut lines: Vec<Line> = Vec::new();
    render_nexus_stream(&mut lines, app, theme);

    // Running tools: white ⏺ with spinner + ToolName(arg)
    for tool in &app.running_tools {
        let elapsed = tool.start_time.elapsed();
        let spinner = theme.spinner.frame_at(elapsed.as_millis());

        let tool_name = to_pascal_case(&tool.name);
        let tool_header = if tool.display_arg.is_empty() {
            tool_name
        } else {
            format!("{tool_name}({})", tool.display_arg)
        };

        lines.push(Line::from(vec![
            Span::styled("⏺ ", Style::default().fg(Color::White)),
            Span::styled(format!("{spinner} "), Style::default().fg(theme.tool)),
            Span::styled(tool_header, Style::default().fg(theme.tool)),
            Span::styled(
                format!(" ({:.1}s)", elapsed.as_secs_f32()),
                Style::default().fg(theme.muted),
            ),
        ]));
    }

    // Calculate visible area and scroll to show most recent (bottom) content
    let visible_height = content_area.height as usize;
    let total_lines = lines.len();

    let max_scroll = total_lines.saturating_sub(visible_height);
    let effective_scroll = app.scroll_offset.min(max_scroll);

    let start_line = if total_lines > visible_height {
        max_scroll.saturating_sub(effective_scroll)
    } else {
        0
    };

    let end_line = (start_line + visible_height).min(total_lines);
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(start_line)
        .take(end_line - start_line)
        .collect();

    let paragraph = Paragraph::new(visible_lines)
        .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::NONE))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, content_area);

    // Scrollbar
    if total_lines > visible_height {
        let scrollbar_position = if max_scroll > 0 {
            max_scroll.saturating_sub(effective_scroll)
        } else {
            0
        };
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scrollbar_position);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme.border));
        frame.render_stateful_widget(scrollbar, content_area, &mut scrollbar_state);
    }
}

fn render_nexus_filter_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let filters = [
        NexusCategory::All,
        NexusCategory::Conversation,
        NexusCategory::Mcp,
        NexusCategory::Security,
        NexusCategory::Audit,
        NexusCategory::Llm,
        NexusCategory::Runtime,
        NexusCategory::Error,
    ];

    let mut spans: Vec<Span> = vec![Span::styled(" Filter: ", Style::default().fg(theme.muted))];

    for f in &filters {
        let is_active = app.nexus_filter == *f;
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
    if let Some(ref agent) = app.nexus_agent_filter {
        spans.push(Span::styled("  Agent: ", Style::default().fg(theme.muted)));
        spans.push(Span::styled(
            agent,
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ));
    }

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}

fn render_agent_selector(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut spans: Vec<Span> = vec![Span::styled(
        " Talking to: ",
        Style::default().fg(theme.muted),
    )];

    for (i, agent) in app.agents.iter().enumerate() {
        let is_focused = app.focused_agent == Some(i);
        let style = if is_focused {
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.assistant)
        };

        if is_focused {
            spans.push(Span::styled("[", Style::default().fg(theme.tool)));
        }
        spans.push(Span::styled(&agent.name, style));
        if is_focused {
            spans.push(Span::styled("]", Style::default().fg(theme.tool)));
        }
        spans.push(Span::raw("  "));
    }

    // Session info
    let msg_count = app.messages.len();
    let total_budget: f64 = app.agents.iter().map(|a| a.budget_spent).sum();
    spans.push(Span::styled("|  ", Style::default().fg(theme.border)));
    spans.push(Span::styled(
        format!("Session: {msg_count} turns"),
        Style::default().fg(theme.muted),
    ));
    spans.push(Span::styled(
        format!("  Budget: ${total_budget:.2}"),
        Style::default().fg(theme.muted),
    ));

    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, area);
}

/// Render the filtered Nexus stream into lines
fn render_nexus_stream<'a>(lines: &mut Vec<Line<'a>>, app: &'a App, theme: &Theme) {
    let effective_filter = if app.agents.len() <= 1 && app.nexus_filter == NexusCategory::All {
        // In single-agent mode with default filter, show only conversation
        NexusCategory::Conversation
    } else {
        app.nexus_filter
    };

    for entry in &app.nexus_stream {
        if !matches_nexus_filter(entry, effective_filter, app.nexus_agent_filter.as_ref()) {
            continue;
        }
        render_nexus_entry(lines, entry, app, theme);
    }
}

/// Check if a `NexusEntry` matches the current filter
fn matches_nexus_filter(
    entry: &NexusEntry,
    filter: NexusCategory,
    agent_filter: Option<&String>,
) -> bool {
    // Category filter
    if filter != NexusCategory::All && entry.category() != filter {
        return false;
    }

    // Agent filter
    if let Some(agent) = agent_filter
        && let Some(entry_agent) = entry.agent_name()
        && entry_agent != agent
    {
        return false;
    }

    true
}

/// Render a single `NexusEntry`
#[allow(clippy::too_many_lines)]
fn render_nexus_entry<'a>(
    lines: &mut Vec<Line<'a>>,
    entry: &'a NexusEntry,
    app: &App,
    theme: &Theme,
) {
    match entry {
        NexusEntry::Message(msg) => {
            // Handle inline tool results
            if let Some(MessageKind::ToolResult(idx)) = &msg.kind {
                render_inline_tool(lines, app, *idx, theme);
            } else {
                match msg.role {
                    MessageRole::User => {
                        for (i, line) in msg.content.lines().enumerate() {
                            if i == 0 {
                                lines.push(Line::from(vec![
                                    Span::styled(
                                        "> ",
                                        Style::default()
                                            .fg(theme.tool)
                                            .add_modifier(Modifier::BOLD),
                                    ),
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
                        let is_diff = msg.kind.is_some();
                        let style = match &msg.kind {
                            Some(MessageKind::DiffHeader | MessageKind::DiffFooter) => {
                                Style::default().fg(theme.diff_context)
                            },
                            Some(MessageKind::DiffRemoved) => {
                                Style::default().fg(theme.diff_removed)
                            },
                            Some(MessageKind::DiffAdded) => Style::default().fg(theme.diff_added),
                            Some(MessageKind::ToolResult(_)) => unreachable!(),
                            None => Style::default()
                                .fg(theme.muted)
                                .add_modifier(Modifier::ITALIC),
                        };
                        let prefix = if is_diff { "  ⎿  " } else { "" };
                        for line in msg.content.lines() {
                            lines.push(Line::from(Span::styled(format!("{prefix}{line}"), style)));
                        }
                    },
                }
            }
            if msg.spacing {
                lines.push(Line::from(""));
            }
        },

        NexusEntry::Event(event) => {
            let event_color = match event.category {
                crate::ui::state::EventCategory::Tool => theme.tool,
                crate::ui::state::EventCategory::Approval => theme.warning,
                crate::ui::state::EventCategory::Error => theme.error,
                crate::ui::state::EventCategory::Session => theme.success,
                crate::ui::state::EventCategory::Security => theme.thinking,
                crate::ui::state::EventCategory::Llm => theme.assistant,
                crate::ui::state::EventCategory::Runtime => theme.muted,
            };

            let elapsed = event.timestamp.elapsed();
            let time_str = format_time(elapsed);

            lines.push(Line::from(vec![
                Span::styled(format!("  {time_str} "), Style::default().fg(theme.muted)),
                Span::styled(
                    format!("[{}] ", event.agent_name),
                    Style::default().fg(theme.tool),
                ),
                Span::styled(
                    format!("{}: ", event.event_type),
                    Style::default().fg(event_color),
                ),
                Span::styled(&event.detail, Style::default().fg(theme.assistant)),
            ]));
        },

        NexusEntry::Approval(approval) => {
            let risk_color = match approval.risk_level {
                RiskLevel::Low => theme.success,
                RiskLevel::Medium => theme.warning,
                RiskLevel::High => theme.error,
            };

            lines.push(Line::from(vec![
                Span::styled(
                    "  🛡 ",
                    Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("[{}] ", approval.agent_name),
                    Style::default().fg(theme.tool),
                ),
                Span::styled(
                    "APPROVAL ",
                    Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("────────────────────", Style::default().fg(theme.border)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("   │ ", Style::default().fg(theme.border)),
                Span::styled(&approval.tool_name, Style::default().fg(theme.tool)),
                Span::styled("  risk: ", Style::default().fg(theme.muted)),
                Span::styled(
                    format!("{:?}", approval.risk_level),
                    Style::default().fg(risk_color).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("   │ ", Style::default().fg(theme.border)),
                Span::styled("[y]", Style::default().fg(theme.success)),
                Span::styled("es ", Style::default().fg(theme.muted)),
                Span::styled("[s]", Style::default().fg(theme.tool)),
                Span::styled("ession ", Style::default().fg(theme.muted)),
                Span::styled("[a]", Style::default().fg(theme.success)),
                Span::styled("lways ", Style::default().fg(theme.muted)),
                Span::styled("[n]", Style::default().fg(theme.error)),
                Span::styled("o  ", Style::default().fg(theme.muted)),
                Span::styled("[→ Shield]", Style::default().fg(theme.muted)),
            ]));
            lines.push(Line::from(Span::styled(
                "   └────────────────────────────────────",
                Style::default().fg(theme.border),
            )));
        },

        NexusEntry::SubAgentLifecycle {
            agent,
            subagent_id,
            action,
            status,
            ..
        } => {
            let status_color = match status {
                SubAgentStatus::Running => theme.tool,
                SubAgentStatus::Completed => theme.success,
                SubAgentStatus::Failed => theme.error,
                SubAgentStatus::TimedOut => theme.warning,
                SubAgentStatus::Cancelled => theme.muted,
            };

            lines.push(Line::from(vec![
                Span::styled("  ⊕ ", Style::default().fg(status_color)),
                Span::styled(format!("[{agent}] "), Style::default().fg(theme.tool)),
                Span::styled(
                    format!("{subagent_id}: {action}"),
                    Style::default().fg(status_color),
                ),
            ]));
        },

        NexusEntry::AuditEntry(audit) => {
            let outcome_color = match audit.outcome {
                AuditOutcome::Success => theme.success,
                AuditOutcome::Failure | AuditOutcome::Violation => theme.error,
                AuditOutcome::Denied => theme.warning,
            };

            let short_hash = if audit.hash.len() > 8 {
                &audit.hash[..8]
            } else {
                &audit.hash
            };

            lines.push(Line::from(vec![
                Span::styled("  ⛓ ", Style::default().fg(theme.muted)),
                Span::styled(format!("#{} ", audit.id), Style::default().fg(theme.muted)),
                Span::styled(
                    format!("[{}] ", audit.agent_name),
                    Style::default().fg(theme.tool),
                ),
                Span::styled(
                    format!("{} → ", audit.action),
                    Style::default().fg(theme.assistant),
                ),
                Span::styled(
                    format!("{:?}", audit.outcome),
                    Style::default().fg(outcome_color),
                ),
                Span::styled(
                    format!(" (hash: {short_hash})"),
                    Style::default().fg(theme.muted),
                ),
            ]));
        },

        NexusEntry::AgentSpawned { name, model, .. } => {
            lines.push(Line::from(vec![
                Span::styled("  ⊕ ", Style::default().fg(theme.tool)),
                Span::styled("Agent spawned: ", Style::default().fg(theme.tool)),
                Span::styled(
                    name,
                    Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" ({model})"), Style::default().fg(theme.muted)),
            ]));
        },

        NexusEntry::SecurityAlert {
            agent,
            detail,
            level,
            ..
        } => {
            let level_color = match level {
                ThreatLevel::Low => theme.success,
                ThreatLevel::Elevated => theme.warning,
                ThreatLevel::High | ThreatLevel::Critical => theme.error,
            };

            lines.push(Line::from(vec![
                Span::styled(
                    "  [!] SECURITY ",
                    Style::default()
                        .fg(level_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("[{agent}]: "), Style::default().fg(theme.tool)),
                Span::styled(
                    detail,
                    Style::default()
                        .fg(level_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        },

        NexusEntry::SystemNotice { content, .. } => {
            lines.push(Line::from(Span::styled(
                content,
                Style::default()
                    .fg(theme.muted)
                    .add_modifier(Modifier::ITALIC),
            )));
        },
    }
}

fn format_time(elapsed: std::time::Duration) -> String {
    let total_secs = elapsed.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{hours:02}:{mins:02}:{secs:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}
