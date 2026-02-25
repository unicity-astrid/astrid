# astrid-telegram

[![Crates.io](https://img.shields.io/crates/v/astrid-telegram)](https://crates.io/crates/astrid-telegram)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Telegram bot frontend and asynchronous client interface for the Astralis daemon.

The `astrid-telegram` crate bridges the Astralis `Frontend` trait to Telegram, exposing interactive agent sessions to mobile devices and remote clients. By connecting to a running `astridd` daemon via WebSocket JSON-RPC, it handles streaming text responses, manages human-in-the-loop (HITL) approval flows via inline keyboards, and ensures secure access control for remote agent administration.

## Core Features

- **Streaming Responses**: Consumes asynchronous `DaemonEvent` chunks, applying a throttled message-editing loop (`EDIT_THROTTLE`) to stream text dynamically without exhausting Telegram API rate limits.
- **Interactive Approval Flows**: Translates `ApprovalRequest` and `ElicitationRequest` events into Telegram inline keyboards. Callback queries map directly to daemon `ApprovalDecision` responses.
- **HTML Boundary Management**: Implements specialized markdown-to-HTML formatting and chunking that prevents mid-tag or mid-entity truncation, ensuring compliance with Telegram's strict 4096-byte message limits.
- **Strict Access Control**: Enforces interaction restrictions via an `allowed_user_ids` configuration, explicitly denying commands from unauthorized users.
- **Dual-Mode Deployment**: Runs as a standalone remote binary (`astrid-telegram`) or as an embedded background task spawned directly by the `astridd` process.

## Architecture

This crate operates as a thin, long-lived client tailored for the Telegram Bot API (`teloxide`). It connects to the core daemon and maps the asynchronous RPC event stream into the Telegram interface.

### The Event Loop

When a user initiates a turn, `astrid-telegram` subscribes to the session's event stream. A dedicated `tokio` task consumes `DaemonEvent` variants:

1. **Text Streaming**: `DaemonEvent::Text` chunks are buffered and periodically flushed to the Telegram API via `edit_message_text`.
2. **Tool Execution**: `DaemonEvent::ToolCallStart` and `ToolCallResult` update a finalized UI block, providing real-time visibility into the agent's remote actions.
3. **Approvals**: `DaemonEvent::ApprovalNeeded` suspends the text stream and dispatches an inline keyboard via the `ApprovalManager`.

### Approval and Elicitation Management

The `ApprovalManager` and `ElicitationManager` maintain thread-safe, time-bounded registries of pending requests. When a user presses an inline button, the Telegram callback payload (`apr:{request_id}:{option_index}`) is resolved against the registry. The manager validates the session context, constructs an `ApprovalDecision`, and resolves the suspended daemon turn over the JSON-RPC client.

## Quick Start

### Standalone Mode

To run the bot as a standalone process connecting to an existing daemon:

1. Configure the bot token and authorized users in `~/.astrid/config.toml`:

```toml
[telegram]
bot_token = "YOUR_TELEGRAM_BOT_TOKEN"
allowed_user_ids = [123456789]
```

2. Start the `astridd` daemon in a separate terminal.
3. Start the Telegram bot:

```bash
cargo run --bin astrid-telegram
```

The bot automatically discovers the daemon port via `~/.astrid/daemon.port` or connects using the optional `daemon_url` configuration parameter.

### Embedded Mode

To run the bot directly inside the daemon process, set `embedded = true` in the configuration:

```toml
[telegram]
bot_token = "YOUR_TELEGRAM_BOT_TOKEN"
embedded = true
allowed_user_ids = [123456789]
```

When the daemon starts, it invokes `astrid_telegram::bot::spawn_embedded` to attach the bot to the running process without requiring a secondary binary.

## Development

Configuration loads primarily through the unified Astralis configuration system (`astrid-config`), checking `~/.astrid/config.toml` or workspace-local files. The system falls back to environment variables when necessary:

- `TELEGRAM_BOT_TOKEN` maps to `bot_token`.
- `TELEGRAM_ALLOWED_USERS` maps to `allowed_user_ids` (comma-separated list).
- `ASTRID_DAEMON_URL` maps to `daemon_url` (optional explicit WebSocket URL).
- `ASTRID_WORKSPACE` maps to `workspace_path` (optional workspace path for session scoping).

For developers integrating this crate into a custom host application, the primary entry points reside in the `bot` module:

- `astrid_telegram::bot::run`: Initializes the `teloxide` dispatcher with a Ctrl+C handler and blocks indefinitely for standalone execution.
- `astrid_telegram::bot::spawn_embedded`: Spawns the dispatcher as an abortable background `tokio` task for daemon integration.

```bash
cargo test -p astrid-telegram
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
