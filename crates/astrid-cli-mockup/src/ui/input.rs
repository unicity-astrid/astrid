//! Input handling for the TUI.

use super::state::{App, SidebarMode, UiState, ViewMode};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::io;

/// Handle input events
pub(crate) fn handle_input(app: &mut App) -> io::Result<()> {
    if let Event::Key(key) = event::read()? {
        // Welcome screen: any key dismisses it (Ctrl+C/D still quits)
        if app.welcome_visible {
            if matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('c' | 'd'), KeyModifiers::CONTROL)
            ) {
                app.should_quit = true;
            } else {
                app.welcome_visible = false;
            }
            return Ok(());
        }

        match app.state {
            UiState::Idle => handle_idle_input(app, key),
            UiState::AwaitingApproval => handle_approval_input(app, key),
            UiState::Thinking { .. } | UiState::Streaming { .. } | UiState::ToolRunning { .. } => {
                handle_interruptible_input(app, key);
            },
            UiState::Interrupted => handle_interrupted_input(app, key),
            UiState::Error { .. } => handle_error_input(app, key),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn handle_idle_input(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Quit: double Ctrl+C to confirm
        (KeyCode::Char('c' | 'd'), KeyModifiers::CONTROL) => {
            if app.quit_pending {
                app.should_quit = true;
            } else {
                app.quit_pending = true;
            }
            return; // Don't reset quit_pending below
        },

        // Submit
        (KeyCode::Enter, _) => app.submit_input(),

        // Number keys 1-9, 0 for direct view jump (only when input is empty)
        (KeyCode::Char(c @ '0'..='9'), KeyModifiers::NONE) if app.input.is_empty() => {
            if let Some(target) = ViewMode::from_number_key(c) {
                switch_to_view(app, target);
            }
        },

        // View-specific keys (only when input is empty, must come before general text input)
        // Nexus view: 'f' to cycle nexus filter
        (KeyCode::Char('f'), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Nexus =>
        {
            app.nexus_filter = app.nexus_filter.next();
        },
        // Nexus view: 'a' to cycle agent filter
        (KeyCode::Char('a'), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Nexus =>
        {
            // Cycle through agents + "all"
            let agent_names: Vec<String> = app.agents.iter().map(|a| a.name.clone()).collect();
            if agent_names.is_empty() {
                app.nexus_agent_filter = None;
            } else {
                match &app.nexus_agent_filter {
                    None => app.nexus_agent_filter = Some(agent_names[0].clone()),
                    Some(current) => {
                        if let Some(idx) = agent_names.iter().position(|n| n == current) {
                            // Safety: saturating_add(1) is fine; if it saturates
                            // the < check will fail and we go to the else branch
                            if idx.saturating_add(1) < agent_names.len() {
                                // Safety: idx + 1 < agent_names.len() checked above
                                #[allow(clippy::arithmetic_side_effects)]
                                let next_idx = idx + 1;
                                app.nexus_agent_filter = Some(agent_names[next_idx].clone());
                            } else {
                                app.nexus_agent_filter = None; // Back to "all"
                            }
                        } else {
                            app.nexus_agent_filter = None;
                        }
                    },
                }
            }
        },
        // Chain view: 'f' to cycle audit filter
        (KeyCode::Char('f'), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Chain =>
        {
            app.audit_filter = app.audit_filter.next();
        },
        // Command view: 's' to cycle sort column
        (KeyCode::Char('s'), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Command =>
        {
            app.command_sort = app.command_sort.next();
        },
        // Command view: 'r' to reverse sort direction
        (KeyCode::Char('r'), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Command =>
        {
            app.command_sort_dir = app.command_sort_dir.toggle();
        },
        // Command view: Space to toggle selection
        (KeyCode::Char(' '), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Command =>
        {
            let idx = app.selected_agent;
            if idx < app.agents.len() {
                if let Some(pos) = app.command_selected.iter().position(|&i| i == idx) {
                    app.command_selected.remove(pos);
                } else {
                    app.command_selected.push(idx);
                }
            }
        },
        // Shield view: y=approve once, s=session, a=always, n=deny (stub)
        (KeyCode::Char('y' | 's' | 'a' | 'n'), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Shield =>
        {
            // Placeholder: will handle focused item approval/deny
        },
        // Shield view: Space to toggle selection
        (KeyCode::Char(' '), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Shield =>
        {
            let idx = app.shield_selected;
            if idx < app.shield_approvals.len() {
                if let Some(pos) = app.shield_selected_items.iter().position(|&i| i == idx) {
                    app.shield_selected_items.remove(pos);
                } else {
                    app.shield_selected_items.push(idx);
                }
            }
        },
        // Shield view: 'f' to cycle risk filter
        (KeyCode::Char('f'), KeyModifiers::NONE)
            if app.input.is_empty() && app.view == ViewMode::Shield =>
        {
            // Cycle risk filter: None -> Low -> Medium -> High -> None
            app.shield_risk_filter = match &app.shield_risk_filter {
                None => Some(super::state::RiskLevel::Low),
                Some(super::state::RiskLevel::Low) => Some(super::state::RiskLevel::Medium),
                Some(super::state::RiskLevel::Medium) => Some(super::state::RiskLevel::High),
                Some(super::state::RiskLevel::High) => None,
            };
        },

        // Text editing
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos = app.cursor_pos.saturating_add(1);
            // Reset scroll to bottom when typing
            app.scroll_offset = 0;
        },
        (KeyCode::Backspace, _) => {
            if app.cursor_pos > 0 {
                // Safety: cursor_pos > 0, so subtraction won't underflow
                #[allow(clippy::arithmetic_side_effects)]
                {
                    app.cursor_pos -= 1;
                }
                app.input.remove(app.cursor_pos);
            }
        },
        (KeyCode::Delete, _) => {
            if app.cursor_pos < app.input.len() {
                app.input.remove(app.cursor_pos);
            }
        },

        // Cursor movement
        (KeyCode::Left, _) => {
            app.cursor_pos = app.cursor_pos.saturating_sub(1);
        },
        (KeyCode::Right, _) => {
            app.cursor_pos = app.cursor_pos.saturating_add(1).min(app.input.len());
        },
        (KeyCode::Home, _) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+Home: scroll to top
            app.scroll_offset = usize::MAX; // Will be clamped in render
        },
        (KeyCode::End, _) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+End: scroll to bottom
            app.scroll_offset = 0;
        },
        (KeyCode::Home, _) => app.cursor_pos = 0,
        (KeyCode::End, _) => app.cursor_pos = app.input.len(),

        // Scrolling
        (KeyCode::PageUp, _) => {
            app.scroll_offset = app.scroll_offset.saturating_add(10);
        },
        (KeyCode::PageDown, _) => {
            app.scroll_offset = app.scroll_offset.saturating_sub(10);
        },
        (KeyCode::Up, _) if app.input.is_empty() => {
            handle_view_up(app);
        },
        (KeyCode::Down, _) if app.input.is_empty() => {
            handle_view_down(app);
        },

        // Clear line
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.input.clear();
            app.cursor_pos = 0;
        },

        // View cycling: Tab (forward), Shift+Tab (backward) when input is empty
        (KeyCode::Tab, _)
            if app.input.is_empty() && !key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            let next = app.view.next();
            switch_to_view(app, next);
        },
        (KeyCode::BackTab, _) if app.input.is_empty() => {
            let prev = app.view.prev();
            switch_to_view(app, prev);
        },

        // Sidebar toggle: Ctrl+B (Expanded ↔ Collapsed ↔ Hidden)
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            app.sidebar = match app.sidebar {
                SidebarMode::Expanded => SidebarMode::Collapsed,
                SidebarMode::Collapsed => SidebarMode::Hidden,
                SidebarMode::Hidden => SidebarMode::Expanded,
            };
        },

        // Toggle tool expansion (Ctrl+E)
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            if let Some(tool) = app.completed_tools.last_mut() {
                tool.expanded = !tool.expanded;
            }
        },

        _ => {},
    }

    // Any key except Ctrl+C/D resets quit confirmation
    app.quit_pending = false;
}

/// Switch to a view, handling auto-sidebar logic
fn switch_to_view(app: &mut App, view: ViewMode) {
    app.view = view;
    // Console mode hides sidebar automatically
    if view == ViewMode::Log {
        app.sidebar = SidebarMode::Hidden;
    } else if app.sidebar == SidebarMode::Hidden {
        app.sidebar = SidebarMode::Expanded;
    }
}

/// Handle Up arrow in view-specific contexts
fn handle_view_up(app: &mut App) {
    match app.view {
        ViewMode::Command => {
            // Navigate agent grid
            if !app.agents.is_empty() {
                app.selected_agent = app.selected_agent.saturating_sub(1);
            }
        },
        ViewMode::Shield => {
            // Navigate within current column
            app.shield_selected = app.shield_selected.saturating_sub(1);
        },
        ViewMode::Chain => {
            // Scroll audit entries up
            app.audit_scroll = app.audit_scroll.saturating_add(1);
        },
        _ => {
            // Default: scroll messages up
            app.scroll_offset = app.scroll_offset.saturating_add(1);
        },
    }
}

/// Handle Down arrow in view-specific contexts
fn handle_view_down(app: &mut App) {
    match app.view {
        ViewMode::Command => {
            if !app.agents.is_empty() {
                app.selected_agent = app
                    .selected_agent
                    .saturating_add(1)
                    .min(app.agents.len().saturating_sub(1));
            }
        },
        ViewMode::Shield => {
            // Flat approval queue navigation
            let max = app.shield_approvals.len().saturating_sub(1);
            app.shield_selected = app.shield_selected.saturating_add(1).min(max);
        },
        ViewMode::Chain => {
            app.audit_scroll = app.audit_scroll.saturating_sub(1);
        },
        _ => {
            app.scroll_offset = app.scroll_offset.saturating_sub(1);
        },
    }
}

fn handle_approval_input(app: &mut App, key: KeyEvent) {
    if app.pending_approvals.is_empty() {
        return;
    }

    let id = app.pending_approvals[app.selected_approval].id;

    match key.code {
        // Approve once
        KeyCode::Char('y' | 'Y') => {
            app.approve_tool(id, false);
        },
        // Approve always (create capability)
        KeyCode::Char('a' | 'A') => {
            app.approve_tool(id, true);
        },
        // Deny
        KeyCode::Char('n' | 'N') | KeyCode::Esc => {
            app.deny_tool(id);
        },
        // Navigate between approvals (if multiple)
        KeyCode::Tab | KeyCode::Down => {
            if !app.pending_approvals.is_empty() {
                // Safety: modulo by len() which is > 0 (checked above), cannot divide by zero
                #[allow(clippy::arithmetic_side_effects)]
                {
                    app.selected_approval =
                        app.selected_approval.saturating_add(1) % app.pending_approvals.len();
                }
            }
        },
        KeyCode::BackTab | KeyCode::Up => {
            if !app.pending_approvals.is_empty() {
                app.selected_approval = app
                    .selected_approval
                    .checked_sub(1)
                    .unwrap_or(app.pending_approvals.len().saturating_sub(1));
            }
        },
        // Interrupt
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            app.should_quit = true;
        },
        _ => {},
    }
}

fn handle_interruptible_input(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Interrupt current operation → show "Interrupted" prompt
        (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Esc, _) => {
            app.state = UiState::Interrupted;
            app.stream_buffer.clear();
        },
        _ => {},
    }
}

fn handle_interrupted_input(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Ctrl+C/D: double-press to quit
        (KeyCode::Char('c' | 'd'), KeyModifiers::CONTROL) => {
            if app.quit_pending {
                app.should_quit = true;
            } else {
                app.quit_pending = true;
            }
            return;
        },
        // Any typing dismisses interrupted state and starts fresh input
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            app.state = UiState::Idle;
            app.input.push(c);
            app.cursor_pos = app.input.len();
        },
        (KeyCode::Enter, _) => {
            app.state = UiState::Idle;
        },
        _ => {
            // Any other key dismisses
            app.state = UiState::Idle;
        },
    }
    app.quit_pending = false;
}

fn handle_error_input(app: &mut App, key: KeyEvent) {
    match key.code {
        // Dismiss error
        KeyCode::Enter | KeyCode::Esc => {
            app.state = UiState::Idle;
        },
        // Quit
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            app.should_quit = true;
        },
        _ => {},
    }
}
