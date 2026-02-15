//! Progress bar / gauge widget for Pulse view.

use crate::ui::theme::Theme;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// Render a gauge bar as a Line. Returns a styled Line.
/// `value` is 0.0..1.0, `width` is the bar width in characters.
pub(crate) fn render_gauge_bar<'a>(
    label: &str,
    value: f32,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    let clamped = value.clamp(0.0, 1.0);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let filled = (clamped * width as f32) as usize;
    let empty = width.saturating_sub(filled);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = (clamped * 100.0) as u8;

    let bar_color = gauge_color(clamped, theme);

    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(theme.muted)),
        Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
        Span::styled("░".repeat(empty), Style::default().fg(theme.border)),
        Span::styled(format!(" {pct}%"), Style::default().fg(bar_color)),
    ])
}

/// Render a budget bar with dollar amounts.
pub(crate) fn render_budget_bar<'a>(
    spent: f64,
    limit: f64,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    #[allow(clippy::cast_possible_truncation)]
    let ratio = if limit > 0.0 {
        (spent / limit).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let filled = (ratio * width as f32) as usize;
    let empty = width.saturating_sub(filled);
    let bar_color = gauge_color(ratio, theme);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = (ratio * 100.0) as u8;

    Line::from(vec![
        Span::styled(
            format!("${spent:.2} / ${limit:.2} "),
            Style::default().fg(theme.assistant),
        ),
        Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
        Span::styled("░".repeat(empty), Style::default().fg(theme.border)),
        Span::styled(format!(" {pct}%"), Style::default().fg(bar_color)),
    ])
}

fn gauge_color(value: f32, theme: &Theme) -> Color {
    if value > 0.8 {
        theme.error
    } else if value > 0.6 {
        theme.warning
    } else {
        theme.success
    }
}
