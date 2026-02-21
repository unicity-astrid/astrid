//! Rendering logic for the TUI.
//!
//! View-specific rendering is delegated to `views/` submodules.
//! Shared helpers (markdown parsing, inline tools, sidebar, etc.) live here.

use super::Theme;
use super::state::{App, RiskLevel, SidebarMode, ToolStatusKind, UiState, ViewMode};
use super::views;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

// ─── Public Helpers (used by views) ──────────────────────────────

/// Convert a line of markdown text to styled spans
pub(super) fn markdown_to_spans<'a>(line: &str, theme: &Theme) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let trimmed = line.trim_start();

    // # Header lines -> bold + user color
    if let Some(rest) = trimmed.strip_prefix("# ") {
        spans.push(Span::styled(
            rest.to_string(),
            Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
        ));
        return spans;
    }
    if let Some(rest) = trimmed.strip_prefix("## ") {
        spans.push(Span::styled(
            rest.to_string(),
            Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
        ));
        return spans;
    }
    if let Some(rest) = trimmed.strip_prefix("### ") {
        spans.push(Span::styled(
            rest.to_string(),
            Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
        ));
        return spans;
    }

    // - list item / * list item -> bullet prefix in tool color
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        // Safety: trimmed is a suffix of line, so line.len() >= trimmed.len()
        #[allow(clippy::arithmetic_side_effects)]
        let indent = &line[..line.len() - trimmed.len()];
        spans.push(Span::styled(
            format!("{indent}  "),
            Style::default().fg(theme.assistant),
        ));
        let rest = &trimmed[2..];
        spans.extend(parse_inline_markdown(rest, theme));
        return spans;
    }

    // Numbered list: 1. item
    if let Some(rest) = trimmed
        .strip_prefix(|c: char| c.is_ascii_digit())
        .and_then(|s| s.strip_prefix(". "))
    {
        // Safety: trimmed is a suffix of line, so line.len() >= trimmed.len()
        #[allow(clippy::arithmetic_side_effects)]
        let indent = &line[..line.len() - trimmed.len()];
        let num_char = trimmed.chars().next().expect("mockup error");
        spans.push(Span::styled(
            format!("{indent}{num_char}. "),
            Style::default().fg(theme.tool),
        ));
        spans.extend(parse_inline_markdown(rest, theme));
        return spans;
    }

    // Regular line: parse inline formatting
    parse_inline_markdown(line, theme)
}

/// Parse inline markdown: **bold** and `code`
fn parse_inline_markdown<'a>(text: &str, theme: &Theme) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut current = String::new();
    let base_style = Style::default().fg(theme.assistant);

    while let Some((i, c)) = chars.next() {
        match c {
            '*' if text[i..].starts_with("**") => {
                if !current.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut current), base_style));
                }
                chars.next();
                let mut bold_text = String::new();
                while let Some((j, bc)) = chars.next() {
                    if bc == '*' && text[j..].starts_with("**") {
                        chars.next();
                        break;
                    }
                    bold_text.push(bc);
                }
                spans.push(Span::styled(
                    bold_text,
                    base_style.add_modifier(Modifier::BOLD),
                ));
            },
            '`' => {
                if !current.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut current), base_style));
                }
                let mut code_text = String::new();
                for (_, cc) in chars.by_ref() {
                    if cc == '`' {
                        break;
                    }
                    code_text.push(cc);
                }
                spans.push(Span::styled(code_text, Style::default().fg(theme.tool)));
            },
            _ => {
                current.push(c);
            },
        }
    }

    if !current.is_empty() {
        spans.push(Span::styled(current, base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }

    spans
}

/// Render a completed tool inline in the message stream.
pub(super) fn render_inline_tool(lines: &mut Vec<Line<'_>>, app: &App, idx: usize, theme: &Theme) {
    let Some(tool) = app.completed_tools.get(idx) else {
        return;
    };

    let bullet_color = match &tool.status {
        ToolStatusKind::Success => theme.success,
        ToolStatusKind::Failed(_) | ToolStatusKind::Denied => theme.error,
        _ => theme.muted,
    };

    let tool_name = to_pascal_case(&tool.name);
    let tool_header = if tool.display_arg.is_empty() {
        tool_name
    } else {
        format!("{tool_name}({})", tool.display_arg)
    };

    lines.push(Line::from(vec![
        Span::styled("⏺ ", Style::default().fg(bullet_color)),
        Span::styled(
            tool_header,
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
    ]));

    match &tool.status {
        ToolStatusKind::Denied => {
            lines.push(Line::from(vec![
                Span::styled("  ⎿ ", Style::default().fg(theme.border)),
                Span::styled("Denied", Style::default().fg(theme.error)),
            ]));
        },
        ToolStatusKind::Failed(err) => {
            lines.push(Line::from(vec![
                Span::styled("  ⎿ ", Style::default().fg(theme.border)),
                Span::styled(err.clone(), Style::default().fg(theme.error)),
            ]));
        },
        _ => {
            if let Some(ref output) = tool.output {
                let output_lines: Vec<&str> = output.lines().collect();
                let max_lines = 20;

                if tool.expanded {
                    for line in output_lines.iter().take(max_lines) {
                        lines.push(Line::from(vec![
                            Span::styled("  ⎿ ", Style::default().fg(theme.border)),
                            Span::styled((*line).to_string(), Style::default().fg(theme.assistant)),
                        ]));
                    }
                    if output_lines.len() > max_lines {
                        // Safety: output_lines.len() > max_lines checked above
                        #[allow(clippy::arithmetic_side_effects)]
                        let extra = output_lines.len() - max_lines;
                        lines.push(Line::from(vec![
                            Span::styled("  ⎿ ", Style::default().fg(theme.border)),
                            Span::styled(
                                format!("... {extra} more lines"),
                                Style::default().fg(theme.warning),
                            ),
                        ]));
                    }
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("  ⎿ ", Style::default().fg(theme.border)),
                        Span::styled(
                            format!("{} lines (ctrl+e to expand)", output_lines.len()),
                            Style::default().fg(theme.muted),
                        ),
                    ]));
                }
            }
        },
    }
}

/// Convert `snake_case` tool names to `PascalCase`: `read_file` → `ReadFile`.
pub(super) fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
            }
        })
        .collect()
}

/// Format a duration as human-readable: "2m 42s" or "3.2s"
pub(super) fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        // Safety: division and modulo by nonzero literal 60
        #[allow(clippy::arithmetic_side_effects)]
        let (mins, rem) = (secs / 60, secs % 60);
        format!("{mins}m {rem:02}s")
    } else {
        format!("{:.1}s", d.as_secs_f32())
    }
}

/// Fun verbs for the thinking spinner (Claude Code style).
pub(crate) const FUN_VERBS: &[(&str, &str)] = &[
    ("Thinking", "Thought"),
    ("Analyzing", "Analyzed"),
    ("Reasoning", "Reasoned"),
    ("Pondering", "Pondered"),
    ("Evaluating", "Evaluated"),
    ("Examining", "Examined"),
    ("Considering", "Considered"),
    ("Processing", "Processed"),
    ("Cogitating", "Cogitated"),
    ("Deliberating", "Deliberated"),
    ("Ruminating", "Ruminated"),
    ("Musing", "Mused"),
    ("Contemplating", "Contemplated"),
    ("Percolating", "Percolated"),
];

// ─── Frame Rendering ─────────────────────────────────────────────

/// Render a frame of the UI
pub(crate) fn render_frame(frame: &mut Frame, app: &App) {
    let theme = Theme::default();

    // Top-level vertical split: main area + full-width status bar
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // Main area (sidebar + content)
            Constraint::Length(1), // Status bar (full width)
        ])
        .split(frame.area());

    let main_area = outer[0];
    let status_area = outer[1];

    // Determine sidebar width based on state
    let sidebar_width = match app.sidebar {
        SidebarMode::Expanded => 20, // Wider for new sections
        SidebarMode::Collapsed => 5,
        SidebarMode::Hidden => 0,
    };

    // Horizontal layout: sidebar + content
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if sidebar_width > 0 {
            vec![Constraint::Length(sidebar_width), Constraint::Min(20)]
        } else {
            vec![Constraint::Min(20)]
        })
        .split(main_area);

    // Render sidebar if visible
    let content_area = if sidebar_width > 0 {
        render_sidebar(frame, h_chunks[0], app, &theme);
        h_chunks[1]
    } else {
        h_chunks[0]
    };

    // Views that don't use the input area (control/monitoring views)
    let no_input_views = matches!(
        app.view,
        ViewMode::Command
            | ViewMode::Topology
            | ViewMode::Shield
            | ViewMode::Pulse
            | ViewMode::Chain
    );

    if no_input_views {
        // Full content area for these views
        match app.view {
            ViewMode::Command => views::render_command(frame, content_area, app, &theme),
            ViewMode::Topology => views::render_topology(frame, content_area, app, &theme),
            ViewMode::Shield => views::render_shield(frame, content_area, app, &theme),
            ViewMode::Pulse => views::render_pulse(frame, content_area, app, &theme),
            ViewMode::Chain => views::render_chain(frame, content_area, app, &theme),
            _ => unreachable!(),
        }
    } else {
        // Dynamic input height based on word-wrapped text
        let dyn_input_h = input_height(app, content_area.width);

        // Vertical layout for content: messages + activity + input
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),              // Messages
                Constraint::Length(1),           // Activity indicator
                Constraint::Length(dyn_input_h), // Input
            ])
            .split(content_area);

        // Render main content area based on current view
        match app.view {
            ViewMode::Nexus => views::render_messages(frame, v_chunks[0], app, &theme),
            ViewMode::Missions => views::render_missions(frame, v_chunks[0], app, &theme),
            ViewMode::Stellar => views::render_stellar(frame, v_chunks[0], app, &theme),
            ViewMode::Log => views::render_log(frame, v_chunks[0], app, &theme),
            _ => unreachable!(),
        }

        // Render pinned activity indicator
        render_activity(frame, v_chunks[1], app, &theme);

        // Render input area
        render_input(frame, v_chunks[2], app, &theme);
    }

    // Render full-width status bar
    render_status(frame, status_area, app, &theme);

    // Render approval overlay if needed
    if app.state == UiState::AwaitingApproval && !app.pending_approvals.is_empty() {
        render_approval_overlay(frame, app, &theme);
    }

    // Render welcome overlay if visible
    if app.welcome_visible {
        render_welcome(frame, content_area, app, &theme);
    }
}

// ─── Sidebar ─────────────────────────────────────────────────────

fn render_sidebar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut items: Vec<ListItem> = Vec::new();
    let is_collapsed = matches!(app.sidebar, SidebarMode::Collapsed);

    // Header
    if !is_collapsed {
        items.push(ListItem::new(Line::from(vec![
            Span::styled(" * ", Style::default().fg(theme.tool)),
            Span::styled(
                "ASTRID",
                Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
            ),
        ])));
        items.push(ListItem::new(Line::from("")));
    }

    // Navigation items grouped by section
    let mut last_section = "";

    for &view in ViewMode::all_ordered() {
        let section = view.section();

        // Section header (only in expanded mode)
        if !is_collapsed && !section.is_empty() && section != last_section {
            if !last_section.is_empty() {
                items.push(ListItem::new(Line::from("")));
            }
            items.push(ListItem::new(Line::from(Span::styled(
                format!(" {section}"),
                Style::default().fg(theme.muted),
            ))));
            last_section = section;
        }

        // Console gets a separator
        if view == ViewMode::Log && !is_collapsed {
            items.push(ListItem::new(Line::from("")));
        }

        let is_selected = app.view == view;
        let style = if is_selected {
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.assistant)
        };

        let name = view.label();
        let key = view.number_key();

        if is_collapsed {
            let letter = &name[..1];
            let icon = if is_selected { ">" } else { " " };
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!("{icon}{letter}"),
                style,
            )])));
        } else {
            let icon = if is_selected { " > " } else { "   " };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(icon, style),
                Span::styled(name, style),
                Span::styled(format!("  ({key})"), Style::default().fg(theme.muted)),
            ])));
        }
    }

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(Color::Gray));

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

// ─── Activity Bar ────────────────────────────────────────────────

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn render_activity(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let spans: Vec<Span> = match &app.state {
        UiState::Thinking { start_time, .. } => {
            let elapsed = start_time.elapsed();
            let spinner = theme.spinner.frame_at(elapsed.as_millis());
            // Safety: division and modulo by nonzero literals cannot panic
            #[allow(clippy::arithmetic_side_effects)]
            let verb_idx = (elapsed.as_millis() / 2500) as usize % FUN_VERBS.len();
            let verb = FUN_VERBS[verb_idx].0;
            // Safety: modulo and division by nonzero literals
            #[allow(clippy::arithmetic_side_effects)]
            let pulse_phase = (elapsed.as_millis() % 2000) as f32 / 2000.0;
            let spinner_color = if pulse_phase < 0.5 {
                theme.thinking
            } else {
                theme.tool
            };

            vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{spinner} "), Style::default().fg(spinner_color)),
                Span::styled(format!("{verb}..."), Style::default().fg(theme.thinking)),
                Span::styled(
                    format!(
                        " ({} · {} tokens)",
                        format_elapsed(elapsed),
                        app.tokens_streamed
                    ),
                    Style::default().fg(theme.muted),
                ),
            ]
        },
        UiState::Streaming { start_time } => {
            let elapsed = start_time.elapsed();
            let spinner = theme.spinner.frame_at(elapsed.as_millis());
            vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{spinner} "), Style::default().fg(theme.success)),
                Span::styled("Responding...", Style::default().fg(theme.success)),
                Span::styled(
                    format!(
                        " ({} · {} tokens)",
                        format_elapsed(elapsed),
                        app.tokens_streamed
                    ),
                    Style::default().fg(theme.muted),
                ),
            ]
        },
        UiState::ToolRunning {
            tool_name,
            start_time,
        } => {
            let elapsed = start_time.elapsed();
            let spinner = theme.spinner.frame_at(elapsed.as_millis());
            let flash_phase = (elapsed.as_millis() % 1000) as f32 / 1000.0;
            let tool_color = if flash_phase < 0.5 {
                theme.tool
            } else {
                theme.warning
            };

            vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{spinner} "), Style::default().fg(tool_color)),
                Span::styled(
                    format!("Running {tool_name}..."),
                    Style::default().fg(theme.tool),
                ),
                Span::styled(
                    format!(" ({})", format_elapsed(elapsed)),
                    Style::default().fg(theme.muted),
                ),
            ]
        },
        UiState::Interrupted => {
            vec![
                Span::styled("  ", Style::default()),
                Span::styled("⏺ Interrupted", Style::default().fg(theme.warning)),
                Span::styled(
                    " · What should Astrid do instead?",
                    Style::default().fg(theme.muted),
                ),
            ]
        },
        UiState::Idle | UiState::AwaitingApproval | UiState::Error { .. } => {
            if let Some((past_verb, duration)) = &app.last_completed
                && let Some(completed_at) = app.last_completed_at
            {
                let since = completed_at.elapsed();
                if since.as_secs() < 8 {
                    let color = if since.as_secs() < 4 {
                        theme.muted
                    } else {
                        theme.border
                    };
                    return frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(
                                format!("{past_verb} for {}", format_elapsed(*duration)),
                                Style::default().fg(color),
                            ),
                        ])),
                        area,
                    );
                }
            }
            vec![]
        },
    };

    if spans.is_empty() {
        frame.render_widget(Paragraph::new(Line::from("")), area);
    } else {
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

// ─── Input Area ──────────────────────────────────────────────────

/// Calculate how many visual lines a text needs when word-wrapped to a given width.
fn wrapped_line_count(text: &str, width: usize) -> usize {
    if text.is_empty() || width == 0 {
        return 1;
    }
    let mut lines = 1usize;
    let mut col = 0usize;
    for word in text.split_whitespace() {
        let wlen = word.len();
        if col > 0 && col.saturating_add(1).saturating_add(wlen) > width {
            lines = lines.saturating_add(1);
            col = wlen;
        } else if col == 0 {
            col = wlen;
        } else {
            col = col.saturating_add(1).saturating_add(wlen);
        }
    }
    lines
}

fn input_height(app: &App, content_width: u16) -> u16 {
    let prompt_len = 2u16;
    let avail = content_width.saturating_sub(prompt_len.saturating_add(1)) as usize;
    let display_text = if app.input.starts_with('/') {
        &app.input[1..]
    } else {
        &app.input
    };
    #[allow(clippy::cast_possible_truncation)]
    let text_lines = wrapped_line_count(display_text, avail) as u16;
    1u16.saturating_add(text_lines).clamp(3, 8)
}

fn render_input(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let is_idle = matches!(app.state, UiState::Idle | UiState::Interrupted);

    // Dashed top border
    let border_line = "╌".repeat(area.width as usize);
    let border = Paragraph::new(Line::from(Span::styled(
        border_line,
        Style::default().fg(theme.border),
    )));
    frame.render_widget(border, Rect::new(area.x, area.y, area.width, 1));

    let input_area = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );

    if app.quit_pending {
        let para = Paragraph::new(Line::from(vec![Span::styled(
            "  Press Ctrl+C again to exit...",
            Style::default().fg(theme.warning),
        )]));
        frame.render_widget(para, input_area);
        return;
    }

    let input_style = if is_idle {
        Style::default().fg(theme.user)
    } else {
        Style::default().fg(theme.muted)
    };

    let prompt = if !is_idle {
        "  "
    } else if app.input.starts_with('/') {
        "/ "
    } else {
        "> "
    };

    let mut text = String::from(prompt);

    if app.input.is_empty() && is_idle {
        text.push_str("█ Ask a question or type / for commands...");
        let para = Paragraph::new(text)
            .style(Style::default().fg(theme.border))
            .wrap(Wrap { trim: false });
        frame.render_widget(para, input_area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prompt, input_style.add_modifier(Modifier::BOLD)),
                Span::styled("█", Style::default().fg(theme.cursor)),
            ])),
            Rect::new(input_area.x, input_area.y, input_area.width, 1),
        );
    } else {
        let display_input = if app.input.starts_with('/') {
            &app.input[1..]
        } else {
            &app.input
        };
        text.push_str(display_input);
        text.push('█');

        let para = Paragraph::new(Line::from(vec![
            Span::styled(prompt, input_style.add_modifier(Modifier::BOLD)),
            Span::styled(display_input.to_string(), input_style),
            Span::styled(
                "█",
                Style::default().fg(if is_idle { theme.cursor } else { theme.border }),
            ),
        ]))
        .wrap(Wrap { trim: false });
        frame.render_widget(para, input_area);
    }
}

// ─── Status Bar ──────────────────────────────────────────────────

fn render_status(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let width = area.width as usize;
    let mut spans: Vec<Span> = Vec::new();

    // Left: breadcrumb path
    let breadcrumb = build_breadcrumb(&app.working_dir);
    for (i, segment) in breadcrumb.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" > ", Style::default().fg(theme.border)));
        }
        // Safety: breadcrumb is non-empty (we're iterating), so len() - 1 is valid
        #[allow(clippy::arithmetic_side_effects)]
        let is_last = i == breadcrumb.len() - 1;
        let style = if is_last {
            Style::default().fg(theme.user)
        } else {
            Style::default().fg(theme.muted)
        };
        spans.push(Span::styled(segment.clone(), style));
    }

    // Center: sandbox / demo status
    let center_text = if !app.sandbox_enabled {
        if app.demo_player.is_some() {
            "!sandbox [DEMO]"
        } else {
            "!sandbox"
        }
    } else if app.demo_player.is_some() {
        "[DEMO]"
    } else {
        ""
    };

    let left_len: usize = spans.iter().map(|s| s.content.len()).sum();
    let center_len = center_text.len();

    // Right: model + context progress bar
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Safety: f32 mul for display estimation
    #[allow(clippy::arithmetic_side_effects)]
    let context_pct = (app.context_usage * 100.0) as u8;
    let bar_width: usize = 8;
    // Safety: division by nonzero literal 100
    #[allow(clippy::arithmetic_side_effects)]
    let filled = (usize::from(context_pct) * bar_width) / 100;
    let empty = bar_width.saturating_sub(filled);

    let bar_color = if context_pct > 80 {
        theme.error
    } else if context_pct > 60 {
        theme.warning
    } else {
        theme.success
    };

    let bar_filled = "█".repeat(filled);
    let bar_empty = "░".repeat(empty);
    let right_label = format!("{} ", app.model_name);
    let right_pct = format!(" {context_pct}%");
    let right_len = right_label
        .len()
        .saturating_add(bar_width)
        .saturating_add(right_pct.len());

    // Safety: division by nonzero literal 2
    #[allow(clippy::arithmetic_side_effects)]
    let center_pos = width / 2;
    // Safety: division by nonzero literal 2
    #[allow(clippy::arithmetic_side_effects)]
    let center_start = center_pos.saturating_sub(center_len / 2);
    let left_pad = center_start.saturating_sub(left_len);
    let right_start = center_start.saturating_add(center_len);
    let right_pad = width.saturating_sub(right_start.saturating_add(right_len));

    spans.push(Span::raw(" ".repeat(left_pad)));

    if !center_text.is_empty() {
        spans.push(Span::styled(
            center_text,
            Style::default().fg(theme.warning),
        ));
    }

    spans.push(Span::raw(" ".repeat(right_pad)));
    spans.push(Span::styled(right_label, Style::default().fg(theme.tool)));
    spans.push(Span::styled(bar_filled, Style::default().fg(bar_color)));
    spans.push(Span::styled(bar_empty, Style::default().fg(theme.border)));
    spans.push(Span::styled(right_pct, Style::default().fg(bar_color)));

    let status = Paragraph::new(Line::from(spans));
    frame.render_widget(status, area);
}

fn build_breadcrumb(path: &str) -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let display = if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };

    let parts: Vec<&str> = display.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return vec![display];
    }

    let start = parts.len().saturating_sub(3);
    let mut result: Vec<String> = parts[start..].iter().map(|s| (*s).to_string()).collect();

    if start > 0 && result.first().is_none_or(|s| s != "~") {
        result.insert(0, "~".to_string());
    }

    result
}

// ─── Overlays ────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn render_welcome(frame: &mut Frame, content_area: Rect, app: &App, theme: &Theme) {
    let box_w = (content_area.width.saturating_sub(4)).min(76);
    let box_h = 14u16.min(content_area.height.saturating_sub(2));
    // Safety: division by nonzero literal 2
    #[allow(clippy::arithmetic_side_effects)]
    let x = content_area
        .x
        .saturating_add((content_area.width.saturating_sub(box_w)) / 2);
    // Safety: division by nonzero literal 2
    #[allow(clippy::arithmetic_side_effects)]
    let y = content_area
        .y
        .saturating_add((content_area.height.saturating_sub(box_h)) / 2);
    let overlay = Rect::new(x, y, box_w, box_h);

    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(" Astrid v0.1.0 ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.tool))
        .style(Style::default().bg(Color::Black));
    frame.render_widget(block, overlay);

    let inner = Rect::new(
        overlay.x.saturating_add(1),
        overlay.y.saturating_add(1),
        overlay.width.saturating_sub(2),
        overlay.height.saturating_sub(2),
    );

    // Safety: division by nonzero literal 2
    #[allow(clippy::arithmetic_side_effects)]
    let half_w = inner.width / 2;
    let left_area = Rect::new(inner.x, inner.y, half_w, inner.height);
    let sep_x = inner.x.saturating_add(half_w);
    let right_area = Rect::new(
        sep_x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(half_w.saturating_add(1)),
        inner.height,
    );

    // Left side: branding
    let display_name = {
        let mut chars = app.username.chars();
        match chars.next() {
            None => "Pilot".to_string(),
            Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
        }
    };

    let left_lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("   Welcome back, {display_name}!"),
            Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "            *",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            "           /|\\",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            "          *-+-*",
            Style::default().fg(theme.tool),
        )),
        Line::from(Span::styled(
            "           \\|/",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            "            *",
            Style::default().fg(theme.muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "       A S T R A L I S",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(&app.model_name, Style::default().fg(theme.muted)),
        ]),
    ];

    let breadcrumb = build_breadcrumb(&app.working_dir);
    let short_path = breadcrumb.join("/");
    let mut left_lines = left_lines;
    left_lines.push(Line::from(vec![
        Span::styled("   ", Style::default()),
        Span::styled(short_path, Style::default().fg(theme.muted)),
    ]));

    let left_para = Paragraph::new(left_lines);
    frame.render_widget(left_para, left_area);

    // Separator
    for row in 0..inner.height {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "│",
                Style::default().fg(theme.border),
            ))),
            Rect::new(sep_x, inner.y.saturating_add(row), 1, 1),
        );
    }

    // Right side
    let sep_len = right_area.width.saturating_sub(2) as usize;
    let mut right_lines: Vec<Line> = vec![
        Line::from(Span::styled(
            " Quick start",
            Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Type a message to begin",
            Style::default().fg(theme.assistant),
        )),
        Line::from(Span::styled(
            " /help for commands",
            Style::default().fg(theme.assistant),
        )),
        Line::from(Span::styled(
            " Tab to switch views",
            Style::default().fg(theme.assistant),
        )),
        Line::from(Span::styled(
            " 1-0 for direct view jump",
            Style::default().fg(theme.assistant),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!(" {}", "─".repeat(sep_len)),
            Style::default().fg(theme.border),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Views: 9",
            Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Nexus Missions Atlas",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            " Command Topology Shield",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            " Chain Pulse Console",
            Style::default().fg(theme.muted),
        )),
    ];

    while right_lines.len() < inner.height as usize {
        right_lines.push(Line::from(""));
    }

    let right_para = Paragraph::new(right_lines);
    frame.render_widget(right_para, right_area);
}

fn render_approval_overlay(frame: &mut Frame, app: &App, theme: &Theme) {
    if app.pending_approvals.is_empty() {
        return;
    }

    let approval = &app.pending_approvals[0];
    let area = frame.area();
    #[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
    // u32 intermediate prevents u16 overflow; result <= area.width so truncation is safe
    let width = (u32::from(area.width) * 60 / 100) as u16;
    let width = width.clamp(40, 60);
    // 2 (tool + risk) + 1 (blank) + 1 (description) + 1 (blank)
    // + details count + 1 (blank) + 1 (hotkeys) + 2 (borders)
    #[allow(clippy::cast_possible_truncation)]
    let content_lines = 9u16.saturating_add(approval.details.len() as u16);
    let height = content_lines.clamp(10, area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let overlay_area = Rect::new(x, y, width, height);

    let risk_color = match approval.risk_level {
        RiskLevel::Low => theme.success,
        RiskLevel::Medium => theme.warning,
        RiskLevel::High => theme.error,
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Tool: ", Style::default().fg(theme.muted)),
            Span::styled(
                &approval.tool_name,
                Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Risk: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{:?}", approval.risk_level),
                Style::default().fg(risk_color),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            &approval.description,
            Style::default().fg(theme.assistant),
        )),
        Line::from(""),
    ];

    for (key, value) in &approval.details {
        lines.push(Line::from(vec![
            Span::styled(format!("{key}: "), Style::default().fg(theme.muted)),
            Span::styled(value, Style::default().fg(theme.assistant)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "[y]",
            Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Allow  "),
        Span::styled(
            "[s]",
            Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Session  "),
        Span::styled(
            "[a]",
            Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Always  "),
        Span::styled(
            "[n]",
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Deny"),
    ]));

    let block = Block::default()
        .title(" Approval Required ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(risk_color))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });

    frame.render_widget(Clear, overlay_area);
    frame.render_widget(paragraph, overlay_area);
}
