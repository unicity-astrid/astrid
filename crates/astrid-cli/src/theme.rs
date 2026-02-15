//! CLI theme and styling.

use colored::Colorize;

/// CLI theme configuration.
pub(crate) struct Theme;

impl Theme {
    /// Format a header.
    pub(crate) fn header(text: &str) -> String {
        format!("{}", text.bold().cyan())
    }

    /// Format a success message.
    pub(crate) fn success(text: &str) -> String {
        format!("{} {}", "✓".green(), text)
    }

    /// Format an error message.
    pub(crate) fn error(text: &str) -> String {
        format!("{} {}", "✗".red(), text.red())
    }

    /// Format a warning message.
    pub(crate) fn warning(text: &str) -> String {
        format!("{} {}", "!".yellow(), text.yellow())
    }

    /// Format an info message.
    pub(crate) fn info(text: &str) -> String {
        format!("{} {}", "i".blue(), text)
    }

    /// Format a dimmed message.
    pub(crate) fn dimmed(text: &str) -> String {
        format!("{}", text.dimmed())
    }

    /// Format a prompt.
    #[allow(dead_code)]
    pub(crate) fn prompt(text: &str) -> String {
        format!("{}", text.bold())
    }

    /// Format a separator line.
    pub(crate) fn separator() -> String {
        "━".repeat(50).dimmed().to_string()
    }

    /// Format a box around text using box-drawing characters.
    pub(crate) fn approval_box(title: &str, content: &str, risk: astrid_core::RiskLevel) -> String {
        let color_fn = match risk {
            astrid_core::RiskLevel::Low => |s: &str| s.green().to_string(),
            astrid_core::RiskLevel::Medium => |s: &str| s.yellow().to_string(),
            astrid_core::RiskLevel::High => |s: &str| s.red().to_string(),
            astrid_core::RiskLevel::Critical => |s: &str| s.red().bold().to_string(),
        };

        let width = 60;
        let top = format!("╭{}╮", "─".repeat(width - 2));
        let bottom = format!("╰{}╯", "─".repeat(width - 2));
        let empty = format!("│{:w$}│", "", w = width - 2);

        let pad_line = |text: &str| -> String {
            // Strip ANSI for length calculation
            let visible_len = strip_ansi(text).len();
            let padding = (width - 4).saturating_sub(visible_len);
            format!("│ {text}{:p$} │", "", p = padding)
        };

        let mut lines = vec![
            color_fn(&top),
            pad_line(&title.bold().to_string()),
            color_fn(&empty),
        ];

        for line in content.lines() {
            lines.push(pad_line(line));
        }

        lines.push(color_fn(&bottom));
        lines.join("\n")
    }

    /// Format a key-value pair for display in approval boxes.
    pub(crate) fn kv(key: &str, value: &str) -> String {
        format!("{}: {}", key.bold(), value)
    }

    /// Format a risk level.
    pub(crate) fn risk_level(level: astrid_core::RiskLevel) -> String {
        match level {
            astrid_core::RiskLevel::Low => "Low".green().to_string(),
            astrid_core::RiskLevel::Medium => "Medium".yellow().to_string(),
            astrid_core::RiskLevel::High => "High".red().to_string(),
            astrid_core::RiskLevel::Critical => "Critical".red().bold().to_string(),
        }
    }

    /// Format a session ID (shortened).
    pub(crate) fn session_id(id: &str) -> String {
        let short = if id.len() > 8 { &id[..8] } else { id };
        format!("{}", short.cyan())
    }

    /// Format a timestamp.
    pub(crate) fn timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
        dt.format("%Y-%m-%d %H:%M").to_string().dimmed().to_string()
    }
}

/// Strip ANSI escape codes from a string for visible-length calculation.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            result.push(c);
        }
    }
    result
}

/// Print a banner for the CLI.
pub(crate) fn print_banner() {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "{}",
        format!(
            r"
   _         _             _ _
  /_\   ___ | |_ _ __ __ _| (_)___
 //_\\ / __|| __| '__/ _` | | / __|
/  _  \\__ \| |_| | | (_| | | \__ \
\_/ \_/|___/ \__|_|  \__,_|_|_|___/
                                   v{version}
"
        )
        .cyan()
    );
    println!("{}", "Secure Agent Runtime".dimmed());
    println!();
}
