# Astralis CLI Mockup - UI Polish Review

## Executive Summary

This review analyzes the current state of the astralis-cli-mockup TUI and identifies areas requiring polish for a production-quality demo experience. The mockup successfully demonstrates core concepts but has several gaps between the data model (ViewMode, tasks, files) and the actual rendering.

**Overall Assessment**: The foundation is solid - the architecture supports views, sidebar states, and rich messaging. However, the rendering layer treats most views identically, and visual differentiation (especially for diffs) is missing.

---

## Critical Issues (Must Fix)

### 1. Diffs Have No Color Coding

**Location**: `demo/player.rs:447-472`

**Current Behavior**:
```
  ┌─ diff: src/auth.rs ─────────────────────
  │ - pass == "admin"                        (gray)
  │ + let user = db.find_user(user)?;        (gray)
  │ + bcrypt::verify(pass, &user.hash)       (gray)
  └───────────────────────────────────────
```

All lines render in `theme.muted` (DarkGray). The `-` and `+` prefixes are present but indistinguishable from surrounding text.

**Expected Behavior**:
- Removed lines (`-`) should be **red** (`theme.diff_removed`)
- Added lines (`+`) should be **green** (`theme.diff_added`)
- Context lines should be gray (`theme.diff_context`)

**Impact**: High - diffs are a core feature for code review workflows

---

### 2. Missions View Shows Text Only (No Kanban Board)

**Location**: `render.rs:14-65`, `state.rs:30-71`

**Current Behavior**:
When switching to Missions view, the demo adds system messages like:
```
  ○ Task: Update authentication docs
  ◐ Task: Add rate limiting tests
```

These render in the general message stream rather than as a visual board.

**Expected Behavior**:
```
┌─ Backlog ────┐ ┌─ Active ─────┐ ┌─ Review ─────┐ ┌─ Complete ───┐
│ ○ Update docs│ │ ◐ Add tests  │ │ ✧ Code review│ │ ★ Fix auth   │
│ ○ Refactor   │ │              │ │              │ │ ★ Add login  │
│              │ │              │ │              │ │              │
└──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘
```

**Root Cause**: `render_frame()` doesn't differentiate between views - it always renders the message stream.

**Impact**: High - Missions view is advertised as a task board but renders identically to Nexus

---

### 3. All Views Render Identically

**Location**: `render.rs:14-65`

**Current Behavior**:
```rust
pub fn render_frame(frame: &mut Frame, app: &App) {
    // ... sidebar width calculation
    // ... layout splitting
    render_messages(frame, v_chunks[0], app, &theme);  // Always same
    render_input(frame, v_chunks[1], app, &theme);
    render_status(frame, v_chunks[2], app, &theme);
}
```

`ViewMode` is stored in `app.view` but never used in rendering decisions.

**Expected Behavior**:
```rust
match app.view {
    ViewMode::Nexus => render_messages(frame, area, app, &theme),
    ViewMode::Missions => render_missions(frame, area, app, &theme),
    ViewMode::Stellar => render_stellar(frame, area, app, &theme),
    ViewMode::Stream => render_stream(frame, area, app, &theme),
    ViewMode::Log => render_log(frame, area, app, &theme),
}
```

**Impact**: High - navigation between views feels broken

---

### 4. No Task or File State

**Location**: `state.rs:30-71`

**Current Behavior**:
`App` struct has no `tasks` field. `AddTask` and `MoveTask` demo steps create system messages but don't maintain actual task state.

**Expected Behavior**:
```rust
pub struct Task {
    pub id: String,
    pub title: String,
    pub column: TaskColumn,
}

pub enum TaskColumn {
    Backlog,
    Active,
    Review,
    Complete,
    Blocked,
}

pub struct App {
    // ... existing fields
    pub tasks: Vec<Task>,
}
```

**Impact**: High - Missions view cannot render a real board without backing data

---

## Polish Issues (Should Fix)

### 5. Tool Output is Hard to Read

**Location**: `render.rs:214`

**Current**:
```rust
Span::styled(*line, Style::default().fg(theme.muted)),
```

Tool output uses `theme.muted` (DarkGray), which is difficult to read on many terminals.

**Recommendation**: Use `theme.assistant` (Gray) for better contrast.

---

### 6. Sidebar Separator Too Subtle

**Location**: `render.rs:120-123`

**Current**:
```rust
items.push(ListItem::new(Line::from(Span::styled(
    " -------------",
    Style::default().fg(theme.muted),
))));
```

The separator blends into the background on dark terminals.

**Recommendation**: Use `Color::Gray` instead of `theme.muted`.

---

### 7. Missing "Allow Session" in Approval Overlay

**Location**: `render.rs:424-431`

**Current**:
```rust
lines.push(Line::from(vec![
    Span::styled("[y]", Style::default().fg(theme.success).add_modifier(Modifier::BOLD)),
    Span::raw(" Allow  "),
    Span::styled("[a]", Style::default().fg(theme.success).add_modifier(Modifier::BOLD)),
    Span::raw(" Always  "),
    Span::styled("[n]", Style::default().fg(theme.error).add_modifier(Modifier::BOLD)),
    Span::raw(" Deny"),
]));
```

Only shows `[y] Allow`, `[a] Always`, `[n] Deny`. Missing `[s] Session` option.

**Recommendation**: Add `[s]` Session option between Allow and Always.

---

### 8. No Diff Colors in Theme

**Location**: `ui/theme.rs`

**Current Theme Fields**:
- user, assistant, muted, tool, success, warning, error, thinking, border, cursor

**Missing**:
- `diff_added: Color::Green`
- `diff_removed: Color::Red`
- `diff_context: Color::DarkGray`
- `file_added: Color::Green`
- `file_modified: Color::Yellow`
- `file_deleted: Color::Red`

---

## Priority Matrix

| Priority | Issue | Effort | Impact |
|----------|-------|--------|--------|
| P0 | Diff colors | Low | High |
| P0 | Theme additions | Low | High |
| P1 | Missions kanban board | Medium | High |
| P1 | Task state in App | Medium | High |
| P1 | View-specific rendering | Medium | High |
| P2 | Session option in approval | Low | Medium |
| P2 | Sidebar separator visibility | Low | Low |
| P2 | Tool output contrast | Low | Medium |

---

## Implementation Plan

### Phase 1: Theme & Colors (P0)
1. Add diff colors to Theme struct
2. Add file status colors to Theme struct
3. Update high_contrast() and light() variants

### Phase 2: Diff Display (P0)
1. Modify ShowDiff handler in player.rs to use styled Message variants
2. Alternative: Create a DiffLine message role or use ANSI codes in content

### Phase 3: Task State & Kanban (P1)
1. Add Task struct and TaskColumn enum to state.rs
2. Add `tasks: Vec<Task>` to App
3. Create `render_missions()` function with column layout
4. Update AddTask/MoveTask handlers to modify task state

### Phase 4: View Differentiation (P1)
1. Add match on `app.view` in `render_frame()`
2. Create placeholder renderers for Stellar, Stream, Log
3. Each view calls appropriate render function

### Phase 5: Minor Polish (P2)
1. Add `[s] Session` to approval overlay
2. Use Color::Gray for sidebar separator
3. Use theme.assistant for tool output

---

## Verification Checklist

```bash
cargo build -p astralis-cli-mockup
cargo run -p astralis-cli-mockup -- --demo showcase
```

- [ ] Diffs show red for removed, green for added
- [ ] Missions view displays column boxes with tasks
- [ ] Switching views changes the main content area
- [ ] Sidebar separator is clearly visible
- [ ] Approval overlay shows [y], [s], [a], [n] options
- [ ] Tool output is readable on dark terminals

---

## Appendix: Proposed Visual Designs

### Diff Display
```
  ┌─ diff: src/auth.rs ─────────────────────
  │ - pass == "admin"                        ← RED
  │ + let user = db.find_user(user)?;        ← GREEN
  │ + bcrypt::verify(pass, &user.hash)       ← GREEN
  └───────────────────────────────────────
```

### Kanban Board (Missions View)
```
┌─ Backlog ────┐ ┌─ Active ─────┐ ┌─ Review ─────┐ ┌─ Complete ───┐
│ ○ Update docs│ │ ◐ Add tests  │ │ ✧ Code review│ │ ★ Fix auth   │
│ ○ Refactor   │ │              │ │              │ │ ★ Add login  │
│              │ │              │ │              │ │              │
└──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘
```

### Approval Overlay with Session
```
┌─────────────── Approval Required ───────────────┐
│ Tool: Bash                                      │
│ Risk: Medium                                    │
│                                                 │
│ Execute shell command in workspace              │
│                                                 │
│ command: npm test                               │
│                                                 │
│ [y] Allow  [s] Session  [a] Always  [n] Deny   │
└─────────────────────────────────────────────────┘
```
