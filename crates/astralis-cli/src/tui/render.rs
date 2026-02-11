//! Rendering logic for the TUI — single-view Nexus layout.

use super::state::{
    App, MessageKind, MessageRole, NexusEntry, PALETTE_MAX_VISIBLE, RiskLevel, ToolStatusKind,
    UiState,
};
use super::theme::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};

// ─── Public Helpers ──────────────────────────────────────────────

/// Convert a line of markdown text to styled spans.
fn markdown_to_spans<'a>(line: &str, theme: &Theme) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let trimmed = line.trim_start();

    // # Header lines → bold + user color
    for prefix in &["### ", "## ", "# "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            spans.push(Span::styled(
                rest.to_string(),
                Style::default().fg(theme.user).add_modifier(Modifier::BOLD),
            ));
            return spans;
        }
    }

    // - list / * list → bullet prefix in tool color
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        let indent = &line[..line.len() - trimmed.len()];
        spans.push(Span::styled(
            format!("{indent}  \u{2022} "),
            Style::default().fg(theme.tool),
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
        let indent = &line[..line.len() - trimmed.len()];
        let num_char = trimmed.chars().next().unwrap();
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

/// Parse inline markdown: **bold** and `code`.
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
fn render_inline_tool(lines: &mut Vec<Line<'_>>, app: &App, idx: usize, theme: &Theme) {
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
                        lines.push(Line::from(vec![
                            Span::styled("  ⎿ ", Style::default().fg(theme.border)),
                            Span::styled(
                                format!("... {} more lines", output_lines.len() - max_lines),
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

/// Convert `snake_case` tool names to `PascalCase`.
fn to_pascal_case(s: &str) -> String {
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

/// Format a duration as human-readable.
fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{:.1}s", d.as_secs_f32())
    }
}

/// Fun verbs for the thinking spinner.
const FUN_VERBS: &[(&str, &str)] = &[
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

// ─── Word Wrapping ───────────────────────────────────────────────

/// Word-wrap a single text line to fit within `max_width` characters.
/// Preserves leading whitespace on the first sub-line so that
/// `markdown_to_spans` can still detect indented list markers.
fn word_wrap_line(text: &str, max_width: usize) -> Vec<String> {
    let char_count = text.chars().count();
    if max_width == 0 || char_count <= max_width {
        return vec![text.to_string()];
    }

    let trimmed = text.trim_start();
    let leading = &text[..text.len() - trimmed.len()];

    let mut result = Vec::new();
    let mut line = String::from(leading);
    let mut width = leading.chars().count();
    let mut first_word = true;

    for word in trimmed.split(' ') {
        if word.is_empty() {
            continue;
        }
        let w = word.chars().count();
        if first_word {
            line.push_str(word);
            width += w;
            first_word = false;
        } else if width + 1 + w <= max_width {
            line.push(' ');
            line.push_str(word);
            width += 1 + w;
        } else {
            result.push(std::mem::take(&mut line));
            line = word.to_string();
            width = w;
        }
    }

    if !line.is_empty() || result.is_empty() {
        result.push(line);
    }

    result
}

// ─── Dynamic Input Height ────────────────────────────────────────

fn wrapped_line_count(text: &str, width: usize) -> usize {
    if text.is_empty() || width == 0 {
        return 1;
    }
    let mut lines = 1;
    let mut col = 0;
    for word in text.split_whitespace() {
        let wlen = word.len();
        if col > 0 && col + 1 + wlen > width {
            lines += 1;
            col = wlen;
        } else if col == 0 {
            col = wlen;
        } else {
            col += 1 + wlen;
        }
    }
    lines
}

fn input_height(app: &App, content_width: u16) -> u16 {
    let prompt_len = 2u16;
    let avail = content_width.saturating_sub(prompt_len + 1) as usize;
    let display_text = if app.input.starts_with('/') {
        &app.input[1..]
    } else {
        &app.input
    };
    #[allow(clippy::cast_possible_truncation)]
    let text_lines = wrapped_line_count(display_text, avail) as u16;
    let base = (1 + text_lines).clamp(3, 8);

    if app.palette_active() {
        let n = app.palette_filtered().len();
        if n > 0 {
            // 1 for separator border + visible item rows
            #[allow(clippy::cast_possible_truncation)]
            let palette_rows = 1 + n.min(PALETTE_MAX_VISIBLE) as u16;
            return base + palette_rows;
        }
    }

    base
}

// ─── Frame Rendering ─────────────────────────────────────────────

/// Render a frame of the TUI.
pub(crate) fn render_frame(frame: &mut Frame, app: &App) {
    let theme = Theme::default();

    // Top-level layout: nexus + activity + input + status bar
    let dyn_input_h = input_height(app, frame.area().width);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),              // Nexus stream
            Constraint::Length(1),           // Activity indicator
            Constraint::Length(dyn_input_h), // Input
            Constraint::Length(1),           // Status bar
        ])
        .split(frame.area());

    render_nexus(frame, chunks[0], app, &theme);
    render_activity(frame, chunks[1], app, &theme);
    render_input(frame, chunks[2], app, &theme);
    render_status(frame, chunks[3], app, &theme);

    // Render approval overlay if needed
    if app.state == UiState::AwaitingApproval && !app.pending_approvals.is_empty() {
        render_approval_overlay(frame, app, &theme);
    }
}

// ─── Nexus Stream ────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn render_nexus(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    // Render messages from the nexus stream
    for entry in &app.nexus_stream {
        match entry {
            NexusEntry::Message(msg) => {
                if let Some(MessageKind::ToolResult(idx)) = &msg.kind {
                    render_inline_tool(&mut lines, app, *idx, theme);
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
                            let mut prev_blank = false;
                            let wrap_width = (area.width as usize).saturating_sub(2);
                            let mut is_first_visual = true;

                            for line in &content_lines {
                                let is_blank = line.trim().is_empty();
                                if is_blank && prev_blank {
                                    continue;
                                }
                                prev_blank = is_blank;

                                if is_blank {
                                    lines.push(Line::from(""));
                                    is_first_visual = false;
                                    continue;
                                }

                                let wrapped = word_wrap_line(line, wrap_width);
                                for sub in &wrapped {
                                    if is_first_visual {
                                        let mut spans = vec![Span::styled(
                                            "⏺ ",
                                            Style::default().fg(Color::White),
                                        )];
                                        spans.extend(markdown_to_spans(sub, theme));
                                        lines.push(Line::from(spans));
                                        is_first_visual = false;
                                    } else {
                                        let mut spans = vec![Span::styled("  ", Style::default())];
                                        spans.extend(markdown_to_spans(sub, theme));
                                        lines.push(Line::from(spans));
                                    }
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
                                Some(MessageKind::DiffAdded) => {
                                    Style::default().fg(theme.diff_added)
                                },
                                Some(MessageKind::ToolResult(_)) => unreachable!(),
                                None => Style::default()
                                    .fg(theme.muted)
                                    .add_modifier(Modifier::ITALIC),
                            };
                            let prefix = if is_diff { "  ⎿  " } else { "" };
                            for line in msg.content.lines() {
                                lines.push(Line::from(Span::styled(
                                    format!("{prefix}{line}"),
                                    style,
                                )));
                            }
                        },
                    }
                }
                if msg.spacing {
                    lines.push(Line::from(""));
                }
            },
        }
    }

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

    // Use Paragraph's built-in scroll so wrapped lines are accounted for.
    let visible_height = area.height as usize;
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total_rows = paragraph.line_count(area.width);
    let max_scroll = total_rows.saturating_sub(visible_height);
    let effective_scroll = app.scroll_offset.min(max_scroll);

    // scroll_offset 0 = bottom, so scroll_y = max_scroll - effective_scroll
    let scroll_y = max_scroll.saturating_sub(effective_scroll);

    #[allow(clippy::cast_possible_truncation)]
    let paragraph = paragraph.scroll((scroll_y as u16, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar
    if total_rows > visible_height {
        let scrollbar_position = max_scroll.saturating_sub(effective_scroll);
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scrollbar_position);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme.border));
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

// ─── Activity Bar ────────────────────────────────────────────────

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn render_activity(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let spans: Vec<Span> = match &app.state {
        UiState::Thinking { start_time, .. } => {
            let elapsed = start_time.elapsed();
            let spinner = theme.spinner.frame_at(elapsed.as_millis());
            let verb_idx = (elapsed.as_millis() / 2500) as usize % FUN_VERBS.len();
            let verb = FUN_VERBS[verb_idx].0;
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
                    " · What should Astralis do instead?",
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

fn render_input(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let is_idle = matches!(app.state, UiState::Idle | UiState::Interrupted);

    // Dashed top border
    let border_line = "╌".repeat(area.width as usize);
    let border = Paragraph::new(Line::from(Span::styled(
        border_line.clone(),
        Style::default().fg(theme.border),
    )));
    frame.render_widget(border, Rect::new(area.x, area.y, area.width, 1));

    // Calculate how much space the palette needs
    let filtered = if app.palette_active() {
        app.palette_filtered()
    } else {
        Vec::new()
    };
    let palette_rows = if filtered.is_empty() {
        0u16
    } else {
        #[allow(clippy::cast_possible_truncation)]
        let rows = 1 + filtered.len().min(PALETTE_MAX_VISIBLE) as u16;
        rows
    };

    let input_area = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1 + palette_rows),
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

    if app.input.is_empty() && is_idle {
        // Placeholder text
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prompt, input_style.add_modifier(Modifier::BOLD)),
                Span::styled("█", Style::default().fg(theme.cursor)),
                Span::styled(
                    " Ask a question or type / for commands...",
                    Style::default().fg(theme.border),
                ),
            ])),
            input_area,
        );
    } else {
        let display_input = if app.input.starts_with('/') {
            &app.input[1..]
        } else {
            &app.input
        };

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

    // Render palette below input
    if !filtered.is_empty() {
        let palette_y = area.y + area.height - palette_rows;

        // Dashed separator between input and palette
        let sep = Paragraph::new(Line::from(Span::styled(
            border_line,
            Style::default().fg(theme.border),
        )));
        frame.render_widget(sep, Rect::new(area.x, palette_y, area.width, 1));

        let items_area = Rect::new(
            area.x,
            palette_y + 1,
            area.width,
            palette_rows.saturating_sub(1),
        );
        render_palette_items(frame, items_area, app, &filtered, theme);
    }
}

fn render_palette_items(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    filtered: &[&super::state::SlashCommandDef],
    theme: &Theme,
) {
    let visible_count = area.height as usize;
    let scroll = app.palette_scroll_offset;

    #[allow(clippy::cast_possible_truncation)]
    for (i, cmd) in filtered.iter().skip(scroll).take(visible_count).enumerate() {
        let is_selected = (scroll + i) == app.palette_selected;
        let y = area.y + i as u16;
        let row_area = Rect::new(area.x, y, area.width, 1);

        let name_style = if is_selected {
            Style::default()
                .fg(theme.tool)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.tool)
        };

        let desc_style = if is_selected {
            Style::default()
                .fg(theme.user)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted)
        };

        let bg_style = if is_selected {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        // Layout: "  /command         Description text…"
        let name_col_width = 18usize;
        let padded_name = format!("  {:<width$}", cmd.name, width = name_col_width);

        let desc_avail = (area.width as usize).saturating_sub(padded_name.len());
        let description = if cmd.description.len() > desc_avail && desc_avail > 1 {
            format!("{}…", &cmd.description[..desc_avail - 1])
        } else {
            cmd.description.to_string()
        };

        // Fill remaining width with background
        let total_used = padded_name.len() + description.len();
        let trailing = (area.width as usize).saturating_sub(total_used);

        let line = Line::from(vec![
            Span::styled(padded_name, name_style),
            Span::styled(description, desc_style),
            Span::styled(" ".repeat(trailing), bg_style),
        ]);

        frame.render_widget(Paragraph::new(line), row_area);
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
        let style = if i == breadcrumb.len() - 1 {
            Style::default().fg(theme.user)
        } else {
            Style::default().fg(theme.muted)
        };
        spans.push(Span::styled(segment.clone(), style));
    }

    // Session ID
    if !app.session_id_short.is_empty() {
        spans.push(Span::styled(
            "  Session: ",
            Style::default().fg(theme.muted),
        ));
        spans.push(Span::styled(
            &app.session_id_short,
            Style::default().fg(theme.tool),
        ));
    }

    // Right: model + context progress bar
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let context_pct = (app.context_usage * 100.0) as u8;
    let bar_width: usize = 8;
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
    let right_len = right_label.len() + bar_width + right_pct.len();

    let left_len: usize = spans.iter().map(|s| s.content.len()).sum();
    let pad = width.saturating_sub(left_len + right_len);

    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right_label, Style::default().fg(theme.tool)));
    spans.push(Span::styled(bar_filled, Style::default().fg(bar_color)));
    spans.push(Span::styled(bar_empty, Style::default().fg(theme.border)));
    spans.push(Span::styled(right_pct, Style::default().fg(bar_color)));

    let status = Paragraph::new(Line::from(spans));
    frame.render_widget(status, area);
}

// ─── Approval Overlay ────────────────────────────────────────────

fn render_approval_overlay(frame: &mut Frame, app: &App, theme: &Theme) {
    if app.pending_approvals.is_empty() {
        return;
    }

    let approval =
        &app.pending_approvals[app.selected_approval.min(app.pending_approvals.len() - 1)];
    let area = frame.area();
    let width = (area.width * 60 / 100).clamp(40, 60);
    // 2 (tool + risk) + 1 (blank) + 1 (description) + 1 (blank)
    // + details count + 1 (blank) + 1 (hotkeys) + 2 (borders)
    #[allow(clippy::cast_possible_truncation)]
    let content_lines = 9 + approval.details.len() as u16;
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
