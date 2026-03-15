# astrid-cli

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)

Command-line interface for the Astrid secure agent runtime.

`astrid-cli` is a thin client: it auto-starts the background kernel daemon if one is not running, performs an authenticated handshake over a Unix Domain Socket, then renders streaming events in a ratatui TUI. All agent logic, LLM calls, MCP tool execution, and security policy enforcement live in the daemon. The CLI's only job is input, routing, and rendering.

## Core Features

- **ratatui TUI** - full-screen terminal interface with a ~60 fps render loop, bracketed paste, Kitty keyboard enhancement protocol, and animated thinking state
- **Daemon auto-start** - spawns the kernel daemon as a subprocess, polls a readiness sentinel file, and connects automatically; cleans up orphan daemons on failure
- **Authenticated IPC** - length-prefixed JSON over a Unix Domain Socket with a `SessionToken`-based handshake before any message traffic
- **Approval gate UI** - renders `ApprovalRequired` payloads from the kernel as interactive risk-coloured prompts (Low / Medium / High / Critical); supports approve-once, approve-for-session, and deny
- **Onboarding flow** - handles `OnboardingRequired` and `ElicitRequest` IPC payloads inline, writing capsule config to `~/.astrid/capsules/<id>/.env.json` with `0o600` permissions, then triggering a kernel reload
- **Selection prompts** - interactive picker for `SelectionRequired` payloads (e.g. model selection)
- **Session history hydration** - replays previous conversation turns from the session store on connect
- **Dynamic slash command palette** - populated at runtime from `GetCommands` kernel API; built-in commands are `/help`, `/clear`, `/install`, `/refresh`, `/quit`
- **Capsule management** - `install`, `update`, `list`, and `deps` subcommands manage the capsule lifecycle outside of a chat session
- **Capsule builder** - `astrid build` auto-detects project type (Rust, TypeScript, MCP server) and packages a `.capsule` archive; accepts `--from-mcp-json` for legacy server migration
- **JSON output mode** - `--format json` switches to NDJSON output for scripting; interactive prompts are auto-denied with diagnostic messages
- **Syntax highlighting** - agent code blocks rendered via `syntect` with base16-ocean.dark theme and 24-bit terminal colour

## Quick Start

```bash
# Install from the workspace root
cargo install --path crates/astrid-cli

# Initialize a workspace in the current directory
astrid init

# Start an interactive session (auto-starts the daemon if needed)
astrid chat

# Resume a specific session
astrid chat --session <UUID>
```

## Commands

| Command | Description |
|---|---|
| `astrid` / `astrid chat` | Start or resume an interactive TUI session |
| `astrid init` | Initialize `.astrid/` workspace directory with a config template |
| `astrid build [path]` | Package a project as a `.capsule` archive |
| `astrid capsule install <source>` | Install a capsule from a local path or GitHub URL |
| `astrid capsule update [name]` | Update one or all installed capsules |
| `astrid capsule list [-v]` | List installed capsules with capability metadata |
| `astrid capsule deps` | Show the resolved capsule dependency graph |
| `astrid session list` | List sessions by last-modified time |
| `astrid session info <id>` | Show session ID and daemon status |
| `astrid session delete <id>` | Delete a session directory |
| `astrid daemon --session <uuid>` | Run the kernel daemon in the foreground for a session |

### Global flags

- `-v` / `--verbose` - enable debug logging
- `--format pretty|json` - output mode (default: `pretty`)

## Architecture

The CLI is intentionally stateless. On `astrid chat`:

1. Resolves (or creates) a session UUID
2. Checks for an existing daemon socket at `~/.astrid/sessions/system.sock`
3. If no socket exists, spawns `astrid daemon` as a subprocess and polls `~/.astrid/sessions/system.ready`
4. Connects and performs the `SessionToken` handshake
5. Sends a `GetCommands` request to populate the slash command palette
6. Runs the ratatui event loop at ~60 fps, interleaving crossterm keyboard events with kernel IPC events

The daemon owns all state. The CLI never writes to the session store directly - it only writes capsule `.env.json` config files during onboarding.

## Development

```bash
cargo test --workspace -- --quiet
```

To run the CLI against a live daemon during development:

```bash
# Terminal 1: start the daemon bound to a test session
cargo run -p astrid-cli -- daemon --session <UUID>

# Terminal 2: connect
cargo run -p astrid-cli -- chat --session <UUID>
```

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE) at your option.
