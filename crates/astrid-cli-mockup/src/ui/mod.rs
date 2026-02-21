//! UI module - terminal interface and rendering.

mod input;
mod render;
pub(crate) mod state;
mod theme;
mod views;
pub(crate) mod widgets;

pub(crate) use input::handle_input;
pub(crate) use render::{FUN_VERBS, render_frame};
pub(crate) use state::{App, Message, MessageRole};

// Re-export types for potential external use
#[allow(unused_imports)]
pub(crate) use state::{ApprovalRequest, ToolStatus, UiState};
pub(crate) use theme::Theme;

use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, backend::TestBackend};
use std::io::{self, Stdout};

/// Type alias for our terminal
pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Initialize the terminal for TUI mode
pub(crate) fn init_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore terminal to normal mode
pub(crate) fn restore_terminal(terminal: &mut Term) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Render a snapshot frame and return it as a string with ANSI colors
pub(crate) fn render_snapshot(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("mockup error");

    terminal
        .draw(|frame| render_frame(frame, app))
        .expect("mockup error");

    // Convert buffer to string with ANSI escape codes for colors
    // Only emit color codes when color changes to reduce verbosity
    let buffer = terminal.backend().buffer();
    let mut output = String::new();
    let mut last_fg = ratatui::style::Color::Reset;

    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];

            // Only emit color code if color changed
            if cell.fg != last_fg {
                let fg = color_to_ansi(cell.fg);
                output.push_str(fg);
                last_fg = cell.fg;
            }

            output.push_str(cell.symbol());
        }
        output.push_str("\x1b[0m\n"); // Reset and newline
        last_fg = ratatui::style::Color::Reset;
    }

    output
}

/// Convert ratatui color to ANSI escape code
fn color_to_ansi(color: ratatui::style::Color) -> &'static str {
    match color {
        ratatui::style::Color::Black => "\x1b[30m",
        ratatui::style::Color::Red => "\x1b[31m",
        ratatui::style::Color::Green => "\x1b[32m",
        ratatui::style::Color::Yellow => "\x1b[33m",
        ratatui::style::Color::Blue => "\x1b[34m",
        ratatui::style::Color::Magenta => "\x1b[35m",
        ratatui::style::Color::Cyan => "\x1b[36m",
        ratatui::style::Color::Gray => "\x1b[37m",
        ratatui::style::Color::DarkGray => "\x1b[90m",
        ratatui::style::Color::LightRed => "\x1b[91m",
        ratatui::style::Color::LightGreen => "\x1b[92m",
        ratatui::style::Color::LightYellow => "\x1b[93m",
        ratatui::style::Color::LightBlue => "\x1b[94m",
        ratatui::style::Color::LightMagenta => "\x1b[95m",
        ratatui::style::Color::LightCyan => "\x1b[96m",
        ratatui::style::Color::White => "\x1b[97m",
        _ => "\x1b[39m",
    }
}
