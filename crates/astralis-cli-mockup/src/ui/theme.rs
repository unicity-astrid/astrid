//! Color theme for the TUI.

use ratatui::style::Color;

/// Spinner animation style
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub(crate) enum SpinnerStyle {
    #[default]
    Stellar, // ✧ ✦ ✶ ✴ ✸ (celestial brand)
    Braille, // ⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏
    Dots,    // ⣾ ⣽ ⣻ ⢿ ⡿ ⣟ ⣯ ⣷
    Orbit,   // ◐ ◓ ◑ ◒
    Pulse,   // ○ ◎ ● ◎
}

impl SpinnerStyle {
    pub(crate) fn frames(self) -> &'static [&'static str] {
        match self {
            SpinnerStyle::Stellar => &["✧", "✦", "✶", "✴", "✸", "✴", "✶", "✦"],
            SpinnerStyle::Braille => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            SpinnerStyle::Dots => &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
            SpinnerStyle::Orbit => &["◐", "◓", "◑", "◒"],
            SpinnerStyle::Pulse => &["○", "◎", "●", "◎"],
        }
    }

    /// Get the current frame for a given elapsed time
    pub(crate) fn frame_at(self, elapsed_ms: u128) -> &'static str {
        let frames = self.frames();
        let interval = 120u128;
        frames[(elapsed_ms / interval % frames.len() as u128) as usize]
    }
}

/// Color theme - works on both light and dark terminals
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct Theme {
    // ── Base colors ──
    /// User input text
    pub user: Color,
    /// Assistant response text
    pub assistant: Color,
    /// Muted/metadata text
    pub muted: Color,
    /// Tool names
    pub tool: Color,
    /// Success indicators
    pub success: Color,
    /// Warning indicators
    pub warning: Color,
    /// Error indicators
    pub error: Color,
    /// Thinking indicator
    pub thinking: Color,
    /// Border color
    pub border: Color,
    /// Cursor color
    pub cursor: Color,

    // ── Diff colors ──
    /// Diff added lines
    pub diff_added: Color,
    /// Diff removed lines
    pub diff_removed: Color,
    /// Diff context lines
    pub diff_context: Color,

    // ── File status colors ──
    /// File added indicator (for Stellar view)
    #[allow(dead_code)]
    pub file_added: Color,
    /// File modified indicator (for Stellar view)
    #[allow(dead_code)]
    pub file_modified: Color,
    /// File deleted indicator (for Stellar view)
    #[allow(dead_code)]
    pub file_deleted: Color,

    // ── Agent status colors ──
    /// Agent ready (green)
    pub agent_ready: Color,
    /// Agent busy (cyan)
    pub agent_busy: Color,
    /// Agent error (red)
    pub agent_error: Color,
    /// Agent paused (yellow)
    pub agent_paused: Color,

    // ── Security / threat colors ──
    /// Threat level low
    pub threat_low: Color,
    /// Threat level elevated
    pub threat_elevated: Color,
    /// Threat level high / critical
    pub threat_high: Color,

    // ── Audit chain colors ──
    /// Chain verified
    pub chain_verified: Color,
    /// Chain broken
    pub chain_broken: Color,

    // ── Capability colors ──
    /// Session-scoped capability
    pub cap_session: Color,
    /// Persistent capability
    pub cap_persistent: Color,
    /// Expired capability
    pub cap_expired: Color,

    /// Spinner animation style
    pub spinner: SpinnerStyle,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            user: Color::White,
            assistant: Color::Gray,
            muted: Color::DarkGray,
            tool: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            thinking: Color::Magenta,
            border: Color::DarkGray,
            cursor: Color::White,
            diff_added: Color::Green,
            diff_removed: Color::Red,
            diff_context: Color::DarkGray,
            file_added: Color::Green,
            file_modified: Color::Yellow,
            file_deleted: Color::Red,

            // Agent status
            agent_ready: Color::Green,
            agent_busy: Color::Cyan,
            agent_error: Color::Red,
            agent_paused: Color::Yellow,

            // Security
            threat_low: Color::Green,
            threat_elevated: Color::Yellow,
            threat_high: Color::Red,

            // Audit
            chain_verified: Color::Green,
            chain_broken: Color::Red,

            // Capabilities
            cap_session: Color::Cyan,
            cap_persistent: Color::Magenta,
            cap_expired: Color::DarkGray,

            spinner: SpinnerStyle::Stellar,
        }
    }
}

impl Theme {
    /// High contrast theme for accessibility
    #[allow(dead_code)]
    pub(crate) fn high_contrast() -> Self {
        Self {
            user: Color::White,
            assistant: Color::White,
            muted: Color::Gray,
            tool: Color::Cyan,
            success: Color::LightGreen,
            warning: Color::LightYellow,
            error: Color::LightRed,
            thinking: Color::LightMagenta,
            border: Color::White,
            cursor: Color::White,
            diff_added: Color::LightGreen,
            diff_removed: Color::LightRed,
            diff_context: Color::Gray,
            file_added: Color::LightGreen,
            file_modified: Color::LightYellow,
            file_deleted: Color::LightRed,
            agent_ready: Color::LightGreen,
            agent_busy: Color::LightCyan,
            agent_error: Color::LightRed,
            agent_paused: Color::LightYellow,
            threat_low: Color::LightGreen,
            threat_elevated: Color::LightYellow,
            threat_high: Color::LightRed,
            chain_verified: Color::LightGreen,
            chain_broken: Color::LightRed,
            cap_session: Color::LightCyan,
            cap_persistent: Color::LightMagenta,
            cap_expired: Color::Gray,
            spinner: SpinnerStyle::Braille,
        }
    }

    /// Light terminal theme
    #[allow(dead_code)]
    pub(crate) fn light() -> Self {
        Self {
            user: Color::Black,
            assistant: Color::DarkGray,
            muted: Color::Gray,
            tool: Color::Blue,
            success: Color::Green,
            warning: Color::Rgb(200, 150, 0), // Darker yellow
            error: Color::Red,
            thinking: Color::Magenta,
            border: Color::Gray,
            cursor: Color::Black,
            diff_added: Color::Green,
            diff_removed: Color::Red,
            diff_context: Color::Gray,
            file_added: Color::Green,
            file_modified: Color::Rgb(200, 150, 0),
            file_deleted: Color::Red,
            agent_ready: Color::Green,
            agent_busy: Color::Blue,
            agent_error: Color::Red,
            agent_paused: Color::Rgb(200, 150, 0),
            threat_low: Color::Green,
            threat_elevated: Color::Rgb(200, 150, 0),
            threat_high: Color::Red,
            chain_verified: Color::Green,
            chain_broken: Color::Red,
            cap_session: Color::Blue,
            cap_persistent: Color::Magenta,
            cap_expired: Color::Gray,
            spinner: SpinnerStyle::Dots,
        }
    }
}
