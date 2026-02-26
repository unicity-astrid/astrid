# astrid-cli

[![Crates.io](https://img.shields.io/crates/v/astrid-cli)](https://crates.io/crates/astrid-cli)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The official command-line interface and terminal frontend for the Astralis secure agent runtime.

`astrid-cli` provides the primary user experience for the Astralis ecosystem. Functioning as a high-performance thin client, it interfaces directly with the `astridd` background daemon to deliver a responsive, rich Terminal User Interface (TUI) without blocking the heavy-lifting of the underlying secure agent runtime.

## Core Features

This crate focuses exclusively on the presentation layer, command parsing, and user interaction mechanics. The actual agent logic, security sandboxing, and session state are managed by the core runtime via the daemon.

### 1. Robust Command Parsing
Powered by `clap`, `astrid-cli` provides a structured, discoverable command surface. It parses user intents, loads local workspace configurations, and routes requests to the appropriate subsystems or daemon endpoints.

### 2. Rich Terminal User Interface (TUI)
Built on top of `ratatui` and `crossterm`, the CLI features a highly responsive, custom-rendered chat interface. It handles raw terminal event streams, smooth scrolling, and dynamic layout adjustments without flickering.

### 3. Native Syntax Highlighting
Code blocks in agent responses are rendered in real-time using `syntect`. The CLI supports 24-bit terminal colors, applying sophisticated themes to source code output before rendering it to the user.

### 4. Seamless Clipboard Integration
Integrating `arboard`, the TUI allows developers to instantly copy generated code snippets or full agent responses directly to the system clipboard, streamlining the workflow between the agent and the IDE.

### 5. Asynchronous Daemon Bridge
To decouple the UI from long-running LLM inferences and tool executions, `astrid-cli` communicates with `astridd` over JSON-RPC (`jsonrpsee`) WebSockets. This means you can close your terminal, reopen it, and immediately reconnect to an ongoing session.

### 6. Strict "Pure Text" Theming
The visual design enforces a strict professional aesthetic. There are zero emojis. All visual communication relies on ANSI color coding, custom ASCII/box-drawing characters for approval prompts, and minimalist animations (like the stellar spinner: `[✧, ✦, ✶, ✴, ✸, ✴, ✶, ✦]`).

## Architecture: Client vs. Daemon

When you run `astrid chat`, the CLI automatically ensures the `astridd` background process is running. 

1. **`astrid` (Client)**: Spawns the TUI, reads keystrokes, highlights syntax, and sends JSON-RPC requests. It maintains zero persistent state.
2. **`astridd` (Daemon)**: Manages MCP servers, executes LLM calls, enforces security policies, and streams events back to the client.

## Quick Start

```bash
# Install the CLI and daemon
cargo install --path crates/astrid-cli

# Start a new interactive session (auto-starts the daemon if needed)
astrid chat
```

### Interactive REPL

Start a fresh conversation or resume an existing one:

```bash
# New session
astrid chat

# Resume a specific session by ID
astrid chat --session <SESSION_ID>
```

**TUI Shortcuts:**
- `Up/Down`: Scroll through conversation history.
- `Ctrl+C`: Exit the interface (the agent continues running in the background).

### Session Management

```bash
# List all active and historical sessions
astrid sessions list

# View the full transcript of a specific session
astrid sessions show <SESSION_ID>
```

### Server and Tool Introspection

Manage the external MCP servers your agent has access to:

```bash
# View configured vs. running servers
astrid servers list
astrid servers running

# Inspect available tools across all active servers
astrid servers tools
```

### Security and Auditing

View the cryptographically verified log of agent actions:

```bash
# List all audit trails
astrid audit list

# View exact parameters of tool calls and capabilities used
astrid audit show <SESSION_ID>
```

## Development

To work on the CLI specifically, ensure you are testing against a local daemon.

```bash
# Run the daemon in the foreground for debugging
cargo run -p astrid-cli --bin astridd -- run --foreground --ephemeral

# In another terminal, run the CLI client with verbose logging
cargo run -p astrid-cli --bin astrid -- -v chat
```

See the root workspace documentation for broader architectural details.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
