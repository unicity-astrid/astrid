//! Threat level indicator widget for Shield view.

use crate::ui::state::ThreatLevel;
use crate::ui::theme::Theme;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// Render a threat level indicator as a Line.
pub(crate) fn render_threat_indicator<'a>(level: ThreatLevel, theme: &Theme) -> Line<'a> {
    let (color, filled) = match level {
        ThreatLevel::Low => (theme.threat_low, 1),
        ThreatLevel::Elevated => (theme.threat_elevated, 2),
        ThreatLevel::High => (theme.threat_high, 3),
        ThreatLevel::Critical => (theme.threat_high, 4),
    };

    let bar: String = "=".repeat(filled) + &" ".repeat(4 - filled);

    Line::from(vec![
        Span::styled("Threat Level: ", Style::default().fg(theme.muted)),
        Span::styled("[", Style::default().fg(theme.border)),
        Span::styled(bar, Style::default().fg(color)),
        Span::styled("] ", Style::default().fg(theme.border)),
        Span::styled(
            level.label(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}
