//! Input handling for the TUI — stripped to Nexus-only keys.

use super::state::{App, ApprovalDecisionKind, PALETTE_MAX_VISIBLE, PendingAction, UiState};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::io;

/// Handle input events.
pub(crate) fn handle_input(app: &mut App) -> io::Result<()> {
    if let Event::Key(key) = event::read()? {
        match app.state {
            UiState::Idle => handle_idle_input(app, key),
            UiState::AwaitingApproval => handle_approval_input(app, key),
            UiState::Thinking { .. } | UiState::Streaming { .. } | UiState::ToolRunning { .. } => {
                handle_interruptible_input(app, key);
            },
            UiState::Interrupted => handle_interrupted_input(app, key),
            UiState::CopyMode => handle_copy_input(app, key),
            UiState::Error { .. } => handle_error_input(app, key),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn handle_idle_input(app: &mut App, key: KeyEvent) {
    let palette_is_active = app.palette_active();

    match (key.code, key.modifiers) {
        // Quit: double Ctrl+C/D to confirm
        (KeyCode::Char('c' | 'd'), KeyModifiers::CONTROL) => {
            if app.quit_pending {
                app.should_quit = true;
            } else {
                app.quit_pending = true;
            }
            return; // Don't reset quit_pending below
        },

        // ── Palette navigation ──────────────────────────────────

        // Enter: select palette command and submit, or normal submit
        (KeyCode::Enter, _) => {
            if palette_is_active {
                let filtered = app.palette_filtered();
                if let Some(cmd) = filtered.get(app.palette_selected) {
                    app.input = cmd.name.to_string();
                    app.cursor_pos = app.input.len();
                }
                app.palette_reset();
            }
            if let Some(content) = app.submit_input() {
                app.pending_actions.push(PendingAction::SendInput(content));
            }
        },

        // Tab: fill input with selected command (no submit, allows appending args)
        (KeyCode::Tab, _) if palette_is_active => {
            let filtered = app.palette_filtered();
            if let Some(cmd) = filtered.get(app.palette_selected) {
                app.input = cmd.name.to_string();
                app.cursor_pos = app.input.len();
            }
            app.palette_reset();
        },

        // Esc: clear input and close palette
        (KeyCode::Esc, _) if palette_is_active => {
            app.input.clear();
            app.cursor_pos = 0;
            app.palette_reset();
        },

        // Up: navigate palette selection
        (KeyCode::Up, _) if palette_is_active => {
            let count = app.palette_filtered().len();
            if count > 0 {
                if app.palette_selected == 0 {
                    app.palette_selected = count - 1;
                } else {
                    app.palette_selected -= 1;
                }
                // Adjust scroll to keep selected visible
                if app.palette_selected < app.palette_scroll_offset {
                    app.palette_scroll_offset = app.palette_selected;
                }
                if app.palette_selected >= app.palette_scroll_offset + PALETTE_MAX_VISIBLE {
                    app.palette_scroll_offset = app.palette_selected + 1 - PALETTE_MAX_VISIBLE;
                }
            }
        },

        // Down: navigate palette selection
        (KeyCode::Down, _) if palette_is_active => {
            let count = app.palette_filtered().len();
            if count > 0 {
                app.palette_selected = (app.palette_selected + 1) % count;
                // Adjust scroll to keep selected visible
                if app.palette_selected >= app.palette_scroll_offset + PALETTE_MAX_VISIBLE {
                    app.palette_scroll_offset = app.palette_selected + 1 - PALETTE_MAX_VISIBLE;
                }
                if app.palette_selected < app.palette_scroll_offset {
                    app.palette_scroll_offset = app.palette_selected;
                }
            }
        },

        // ── Copy mode ────────────────────────────────────────────
        // Ctrl+Shift+C enters copy mode (capital C = Shift held)
        (KeyCode::Char('C'), m) if m.contains(KeyModifiers::CONTROL) => {
            app.enter_copy_mode();
        },

        // ── Text editing ────────────────────────────────────────
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos += c.len_utf8();
            app.scroll_offset = 0;
            app.palette_reset();
        },
        (KeyCode::Backspace, _) => {
            if app.cursor_pos > 0 {
                let prev = app.input[..app.cursor_pos]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i);
                app.input.remove(prev);
                app.cursor_pos = prev;
            }
            app.palette_reset();
        },
        (KeyCode::Delete, _) => {
            if app.cursor_pos < app.input.len() {
                app.input.remove(app.cursor_pos);
            }
            app.palette_reset();
        },

        // Cursor movement
        (KeyCode::Left, _) => {
            if app.cursor_pos > 0 {
                app.cursor_pos = app.input[..app.cursor_pos]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i);
            }
        },
        (KeyCode::Right, _) => {
            if app.cursor_pos < app.input.len() {
                let (_, c) = app.input[app.cursor_pos..].char_indices().next().unwrap();
                app.cursor_pos += c.len_utf8();
            }
        },
        (KeyCode::Home, _) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_offset = usize::MAX;
        },
        (KeyCode::End, _) if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
            app.scroll_offset = app.scroll_offset.saturating_add(1);
        },
        (KeyCode::Down, _) if app.input.is_empty() => {
            app.scroll_offset = app.scroll_offset.saturating_sub(1);
        },

        // Clear line
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.input.clear();
            app.cursor_pos = 0;
            app.palette_reset();
        },

        // Toggle last tool expansion
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

fn handle_approval_input(app: &mut App, key: KeyEvent) {
    if app.pending_approvals.is_empty() {
        return;
    }

    let id = app.pending_approvals[app.selected_approval.min(app.pending_approvals.len() - 1)]
        .id
        .clone();

    match key.code {
        // Approve once
        KeyCode::Char('y' | 'Y') => {
            app.approve_tool(&id, ApprovalDecisionKind::Once);
        },
        // Approve session
        KeyCode::Char('s' | 'S') => {
            app.approve_tool(&id, ApprovalDecisionKind::Session);
        },
        // Approve always
        KeyCode::Char('a' | 'A') => {
            app.approve_tool(&id, ApprovalDecisionKind::Always);
        },
        // Deny
        KeyCode::Char('n' | 'N') | KeyCode::Esc => {
            app.deny_tool(&id);
        },
        // Navigate between approvals
        KeyCode::Tab | KeyCode::Down => {
            if !app.pending_approvals.is_empty() {
                app.selected_approval = (app.selected_approval + 1) % app.pending_approvals.len();
            }
        },
        KeyCode::BackTab | KeyCode::Up => {
            if !app.pending_approvals.is_empty() {
                app.selected_approval = app
                    .selected_approval
                    .checked_sub(1)
                    .unwrap_or(app.pending_approvals.len() - 1);
            }
        },
        // Quit
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            app.should_quit = true;
        },
        _ => {},
    }
}

fn handle_interruptible_input(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Esc, _) => {
            app.state = UiState::Interrupted;
            app.stream_buffer.clear();
            app.pending_actions.push(PendingAction::CancelTurn);
        },
        _ => {},
    }
}

fn handle_interrupted_input(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c' | 'd'), KeyModifiers::CONTROL) => {
            if app.quit_pending {
                app.should_quit = true;
            } else {
                app.quit_pending = true;
            }
            return;
        },
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            app.state = UiState::Idle;
            app.input.push(c);
            app.cursor_pos = app.input.len();
        },
        _ => {
            app.state = UiState::Idle;
        },
    }
    app.quit_pending = false;
}

fn handle_copy_input(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Navigate up
        (KeyCode::Up, _) => {
            if app.copy_cursor > 0 {
                app.copy_cursor -= 1;
            }
        },
        // Navigate down
        (KeyCode::Down, _) => {
            if app.copy_cursor + 1 < app.nexus_stream.len() {
                app.copy_cursor += 1;
            }
        },
        // Jump up by 5
        (KeyCode::PageUp, _) => {
            app.copy_cursor = app.copy_cursor.saturating_sub(5);
        },
        // Jump down by 5
        (KeyCode::PageDown, _) => {
            app.copy_cursor = (app.copy_cursor + 5).min(app.nexus_stream.len().saturating_sub(1));
        },
        // Toggle selection on current entry
        (KeyCode::Char(' '), _) => {
            app.toggle_copy_selection();
        },
        // Select all
        (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
            app.select_all_copy();
        },
        // Copy and exit
        (KeyCode::Enter, _) => {
            match app.copy_to_clipboard() {
                Ok(()) => {
                    app.copy_notice = Some(("Copied!".to_string(), std::time::Instant::now()));
                },
                Err(e) => {
                    app.copy_notice = Some((e, std::time::Instant::now()));
                },
            }
            app.exit_copy_mode();
        },
        // Cancel / quit copy mode
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.exit_copy_mode();
        },
        _ => {},
    }
}

fn handle_error_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            app.state = UiState::Idle;
        },
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            app.should_quit = true;
        },
        _ => {},
    }
}
