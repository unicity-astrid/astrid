# astrid-cli-mockup

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

UI/UX prototype for the Astrid interactive CLI experience.

A self-contained TUI mockup built on [ratatui](https://github.com/ratatui/ratatui) and
[crossterm](https://github.com/crossterm-rs/crossterm). No LLM calls, no database, no runtime
dependencies - every agent action, tool execution, and approval flow is simulated. Used to design
and validate the full dashboard experience before wiring it to the real Astrid gateway.

## Views

The dashboard is organized into three sections, navigated by number keys `1`-`8` and `0`, or
`Tab`/`Shift+Tab`:

**OPERATE**
- `1` **Nexus** - Unified chronological stream of conversation, tool calls, security events, audit
  entries, and sub-agent lifecycle events. Filterable by category (Chat, MCP, Security, Audit, LLM,
  Runtime, Error) and by agent.
- `2` **Missions** - Kanban task board with columns: Backlog, Active, Review, Complete, Queued.
  Tasks are optionally assigned to a named agent.
- `3` **Atlas** - File explorer showing workspace files with change-status indicators (Unchanged,
  Modified, Added, Deleted, Editing).

**CONTROL**
- `4` **Command** - Agent table with sortable columns (Name, Status, Activity, Budget, Sub-agents,
  Context%) and multi-select for bulk operations.
- `5` **Topology** - Agent hierarchy tree showing parent agents and their spawned sub-agents with
  depth indentation, status, and timing.
- `6` **Shield** - Prioritized approval queue. Displays pending tool approvals grouped by risk
  level (Low/Medium/High), active capability tokens, and recent denials. Shows current threat level
  (Low/Elevated/High/Critical).

**MONITOR**
- `7` **Chain** - Live audit trail with integrity verification. Filterable by category (Security,
  Tools, Sessions, LLM). Each entry shows agent, action, auth method, outcome, and a hash.
- `8` **Pulse** - Health, budget, and performance dashboard. Shows per-component health checks,
  session spend vs. limit, token counts, burn rate, tool latency, LLM latency, and events per
  minute.
- `0` **Console** - Minimal log view.

## Demo Scenarios

Ten scripted scenarios play automatically - no user input needed. Each scenario drives the `App`
state machine through typed input, agent thinking/streaming, tool requests, approvals, and view
transitions.

| Scenario | Description |
|---|---|
| `simple-qa` | Question and answer with no tool calls |
| `file-read` | Agent reads a workspace file |
| `file-write` | Agent writes a file, triggers approval dialog |
| `multi-tool` | Agent chains multiple tool calls in sequence |
| `approval-flow` | Exercises all approval choices (Once, Always, Session, Deny) |
| `error` | Tool fails, agent recovers |
| `full-demo` | End-to-end showcase |
| `showcase` | Boot sequence, all views, every feature |
| `quick` | 30-second highlight reel |
| `multi-agent-ops` | Three agents, sub-agents, approvals, and audit chain |

## Quick Start

```bash
# Interactive mode (defaults to Nexus view, no demo loaded)
cargo run -p astrid-cli-mockup

# Start with a scripted demo playing automatically
cargo run -p astrid-cli-mockup -- --demo showcase

# Non-interactive snapshot mode - prints rendered frames to stdout
cargo run -p astrid-cli-mockup -- --snapshot showcase --steps 5
```

## Keybindings

| Key | Action |
|---|---|
| `1`-`8`, `0` | Jump directly to a view (when input is empty) |
| `Tab` / `Shift+Tab` | Cycle views forward/backward |
| `f` | Cycle Nexus stream filter (Nexus view, input empty) |
| `a` | Cycle agent filter (Nexus view, input empty) |
| `s` | Cycle sort column (Command view) |
| `Enter` | Submit input / confirm selection |
| `y` / `n` | Approve / deny pending tool call |
| `Esc` | Interrupt thinking or streaming |
| `Ctrl+C` (twice) | Quit |
| `/clear` | Clear messages and event stream |
| `/demo <scenario>` | Load a demo scenario at runtime |
| `/help` | Show available slash commands |

## Architecture

```
src/
  main.rs              - Entry point, CLI arg parsing, snapshot mode
  demo/
    scenarios.rs       - DemoStep enum and DemoScenario struct
    scenarios/         - One file per named scenario (10 total)
    player.rs          - DemoPlayer: advances steps against App state each frame
  mock/
    responses.rs       - Canned LLM response strings
    tools.rs           - Mock tool-call extraction from [TOOL:name:arg] patterns
  ui/
    state.rs           - App struct, all view state, UiState machine, message types
    input.rs           - Keyboard event routing per UiState
    render.rs          - Frame renderer, markdown-to-spans, inline tool rendering
    theme.rs           - Theme struct (default, high-contrast, light) + SpinnerStyle
    views/             - One renderer per view (nexus, missions, stellar, command,
                         topology, shield, chain, pulse, log)
    widgets/           - Shared widgets (agent_card, gauge, threat indicator,
                         ticker, tree node)
```

The `DemoPlayer` owns a `DemoScenario` and calls `advance(&mut app)` once per frame from the main
loop. Steps with timing (Pause, UserTypes, AgentStreams) check elapsed time against a step-local
`Instant`; fast-forward mode skips all delays for snapshot use.

## Development

```bash
cargo test --workspace -- --quiet
```

This crate has `publish = false` and is not released to crates.io.

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
