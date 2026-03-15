# astrid-cli

[![Crates.io](https://img.shields.io/crates/v/astrid-cli)](https://crates.io/crates/astrid-cli)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The terminal frontend for the Astrid OS.**

In the OS model, this is a shell. It does not run agents, call LLMs, enforce security policy, or own state. It spawns the kernel daemon as a background process, connects over a Unix domain socket, and renders streaming events in a ratatui TUI. Kill the CLI. Reconnect. Pick up where you left off. The daemon owns the session. The CLI is disposable.

## Why it exists

Every kernel needs a way for the operator to interact with it. `astrid-cli` is that first interface. It auto-starts the kernel, performs an authenticated handshake (`SessionToken` over length-prefixed JSON), and then acts as a dumb pipe: user input goes in, streaming events come back, the TUI renders them.

Separating the CLI from the kernel means the kernel survives disconnection. It also means the CLI can be replaced. A web UI, a Discord bot, or a programmatic NDJSON consumer can all connect to the same daemon socket. The CLI is the reference implementation.

## What it does

- **Auto-starts the kernel** on `astrid chat`. Spawns the daemon subprocess, polls a readiness sentinel, connects automatically. Kills orphan daemons on failure.
- **Authenticated IPC.** Length-prefixed JSON over Unix domain socket with `SessionToken` handshake before any message traffic.
- **ratatui TUI.** Full-screen terminal interface with bracketed paste, Kitty keyboard protocol (where supported), and animated thinking state.
- **Approval gate rendering.** Renders `ApprovalRequired` payloads as interactive risk-colored prompts (Low/Medium/High/Critical). Approve once, approve for session, or deny.
- **Onboarding flow.** Handles `OnboardingRequired` and `ElicitRequest` payloads inline, writes capsule config with `0o600` permissions, triggers kernel reload.
- **Capsule management.** `install`, `update`, `list`, and `deps` subcommands manage capsules outside a chat session.
- **Capsule builder.** `astrid build` auto-detects project type (Rust, TypeScript, MCP server) and packages a `.capsule` archive.
- **JSON output mode.** `--format json` switches to NDJSON for scripting. Interactive prompts are auto-denied with diagnostic messages.
- **Syntax highlighting.** Code blocks rendered via `syntect` with base16-ocean.dark and 24-bit color.

## Subcommands

| Command | What it does |
|---|---|
| `astrid chat` | Start or resume an interactive session. `--session <UUID>` to resume. |
| `astrid init` | Initialize a workspace in the current directory. |
| `astrid build` | Package a capsule. `--from-mcp-json` for legacy migration. |
| `astrid capsule install <source>` | Install from local path or GitHub URL. `--workspace` for workspace-level. |
| `astrid capsule update [target]` | Update one or all capsules. |
| `astrid capsule list` | List installed capsules with capability metadata. |
| `astrid capsule deps` | Print the capsule dependency graph. |
| `astrid session list/delete/info` | Manage persisted sessions in `~/.astrid/sessions/`. |
| `astrid daemon` | Run the kernel daemon directly (normally auto-spawned by `chat`). |

## Quick start

```bash
cargo install --path crates/astrid-cli

astrid init
astrid chat
```

## Development

```bash
cargo test -p astrid-cli
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
