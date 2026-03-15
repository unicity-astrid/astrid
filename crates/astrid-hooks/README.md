# astrid-hooks

[![Crates.io](https://img.shields.io/crates/v/astrid-hooks)](https://crates.io/crates/astrid-hooks)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)

Hook system for the Astrid secure agent runtime.

`astrid-hooks` lets you intercept and act on runtime lifecycle events - tool calls, approvals, session transitions, compaction, and more - without modifying the core engine. Hooks dispatch to shell commands, HTTP webhooks, or Extism-based WASM modules. Each hook returns a typed result (`Continue`, `Block`, `Ask`, or `ContinueWith`) that the runtime uses to gate or modify the triggering operation.

## Core Features

- **23 lifecycle events** covering the full execution arc: session start/end/reset, prompt assembly, tool calls, approval flows, context compaction, subagent operations, and kernel lifecycle.
- **Three active handler types:** shell commands (with sandboxed env), HTTP webhooks (via `curl`), and Extism WASM modules using the shared capsule ABI.
- **Typed hook results:** handlers signal `Continue`, `Block` (with reason), `Ask` (with question), or `ContinueWith` (with context modifications). Chained results follow deterministic precedence - any `Block` short-circuits the chain.
- **Priority-ordered execution:** hooks on the same event are sorted by integer priority before dispatch. Lower values run first.
- **Configurable failure policy:** each hook declares a `FailAction` - `Warn` (continue), `Block` (halt chain), or `Ignore` (silent).
- **Command sandboxing:** the command handler clears the subprocess environment, allows only a safe allowlist (`PATH`, `HOME`, `USER`, etc.), restricts `PATH` to standard system directories, and filters dangerous variables via `astrid-core`'s env policy blocklist.
- **Context injection:** every handler receives the invocation context as environment variables (`ASTRID_HOOK_EVENT`, `ASTRID_SESSION_ID`, `ASTRID_HOOK_DATA`, etc.) and as full context JSON on stdin.
- **TOML discovery:** hooks can be loaded from `HOOK.toml` / `hooks.toml` files in `.astrid/hooks/` or any configured extra path.
- **Built-in profiles:** four named profiles (`minimal`, `logging`, `security`, `development`) provide ready-to-use hook sets for common scenarios.

## Quick Start

```toml
[dependencies]
astrid-hooks = "0.2.0"
```

```rust
use astrid_hooks::prelude::*;

// Hook::new, HookEvent, HookHandler, and HookResult
// are the public API surface. HookManager and HookExecutor
// are crate-internal and driven by the Astrid runtime.

// Hooks are defined as values and registered with the runtime's
// internal HookManager.
let audit_hook = Hook::new(HookEvent::PreToolCall)
    .with_name("audit-tool-calls")
    .with_handler(HookHandler::Command {
        command: "audit-logger".to_string(),
        args: vec![],
        env: Default::default(),
        working_dir: None,
    })
    .with_timeout(10)
    .with_fail_action(FailAction::Warn);
```

To fire-and-forget without blocking the triggering operation:

```rust
let background_hook = Hook::new(HookEvent::SessionStart)
    .with_handler(HookHandler::Http {
        url: "https://hooks.example.com/session".to_string(),
        method: "POST".to_string(),
        headers: Default::default(),
        body_template: Some(
            r#"{"event": "{{event}}", "session": "{{session_id}}"}"#.to_string()
        ),
    })
    .async_mode();
```

To use a WASM module that implements the capsule ABI (`run-hook` export):

```rust
let wasm_hook = Hook::new(HookEvent::PreApproval)
    .with_handler(HookHandler::Wasm {
        module_path: ".astrid/hooks/policy-check.wasm".to_string(),
        function: "run-hook".to_string(),
    })
    .with_fail_action(FailAction::Block);
```

## API Reference

### Key Types

**`Hook`** - a hook definition. Constructed with `Hook::new(event)` and configured through a builder chain.

| Field | Type | Default | Description |
|---|---|---|---|
| `id` | `Uuid` | random | Unique identifier |
| `name` | `Option<String>` | `None` | Human-readable label |
| `event` | `HookEvent` | required | Triggering event |
| `matcher` | `Option<HookMatcher>` | `None` | Optional filter (glob, regex, tool names, server names) |
| `handler` | `HookHandler` | required | Execution backend |
| `timeout_secs` | `u64` | `30` | Per-hook timeout |
| `fail_action` | `FailAction` | `Warn` | Behavior on handler failure |
| `async_mode` | `bool` | `false` | Fire-and-forget (don't block) |
| `enabled` | `bool` | `true` | Whether the hook is active |
| `priority` | `i32` | `100` | Execution order (lower = earlier) |

**`HookEvent`** - the 23 lifecycle points where hooks can fire:

```
SessionStart / SessionEnd / SessionReset
UserPrompt / MessageReceived / MessageSend / MessageSent
PreToolCall / PostToolCall / ToolError / ToolResultPersist
PreApproval / PostApproval
Notification
PreCompact / PostCompact
PromptBuild
ModelResolve
AgentLoopEnd
SubagentStart / SubagentStop
KernelStart / KernelStop
```

**`HookHandler`** - the execution backend for a hook:

- `HookHandler::Command { command, args, env, working_dir }` - spawns a subprocess. The command receives context env vars and full context JSON on stdin. Stdout is parsed as the `HookResult`.
- `HookHandler::Http { url, method, headers, body_template }` - dispatches via `curl`. Template variables like `{{event}}`, `{{session_id}}`, and `{{tool_name}}` are JSON-escaped before substitution to prevent injection. Response body is parsed as the `HookResult`.
- `HookHandler::Wasm { module_path, function }` - loads an Extism WASM module (cached after first load) and calls the named export with a serialized `CapsuleAbiContext`. The module returns a `CapsuleAbiResult` whose `action` field maps to `HookResult`. WASM hooks share host functions with the capsule engine but do not receive global filesystem access or an identity store.
- `HookHandler::Agent { prompt_template, model, max_tokens }` - **stubbed, not implemented.** Always returns `Skipped`. Do not use in production.

**`HookResult`** - what a handler signals back to the runtime:

- `Continue` - proceed normally
- `ContinueWith { modifications }` - proceed with modified context key-value pairs
- `Block { reason }` - halt the operation with a reason string
- `Ask { question, default }` - surface a question to the user before proceeding

**`HookMatcher`** - optional filter on a hook:

- `Glob { pattern }` - matches `tool_name` from context data against a glob pattern
- `Regex { pattern }` - matches `tool_name` against a regex
- `ToolNames { names }` - exact match against a list of tool names
- `ServerNames { names }` - exact match against a list of server names

### Output Protocol for Command and HTTP Handlers

Handlers signal their result through stdout (command) or response body (HTTP). The parser accepts:

- Empty or whitespace: `Continue`
- `"continue"` (case-insensitive): `Continue`
- `"block: <reason>"`: `Block` with the given reason
- `"ask: <question>"`: `Ask` with the given question
- JSON object with an `"action"` field: deserialized directly into `HookResult`

### Context Environment Variables

The command handler injects these into every subprocess:

| Variable | Content |
|---|---|
| `ASTRID_HOOK_ID` | Invocation UUID |
| `ASTRID_HOOK_EVENT` | Event name (e.g. `pre_tool_call`) |
| `ASTRID_HOOK_TIMESTAMP` | RFC 3339 timestamp |
| `ASTRID_SESSION_ID` | Session UUID (if set) |
| `ASTRID_USER_ID` | User UUID (if set) |
| `ASTRID_HOOK_DATA` | Event-specific data as a JSON string |

### TOML Hook Files

Hooks can be written to disk and discovered automatically. Place `HOOK.toml`, `hook.toml`, or `hooks.toml` files under `.astrid/hooks/` in the workspace root (or pass additional paths to `discover_hooks`).

Example `HOOK.toml`:

```toml
id = "550e8400-e29b-41d4-a716-446655440000"
event = "pre_tool_call"
name = "block-rm"
timeout_secs = 5
fail_action = "block"

[handler]
type = "command"
command = "sh"
args = ["-c", "case \"$ASTRID_HOOK_DATA\" in *rm*) echo 'block: rm blocked'; ;; *) echo continue; ;; esac"]
```

### Built-in Profiles

Four named profiles are available for quick configuration:

| Profile | Description |
|---|---|
| `minimal` | No hooks |
| `logging` | Async `echo` hooks on `SessionStart`, `SessionEnd`, `PreToolCall` |
| `security` | Blocking shell script on `PreToolCall` that rejects `rm`, `sudo`, `chmod`, `chown`, `mkfs`, `dd` |
| `development` | Async append-to-logfile hooks on `PreToolCall` and `ToolError` |

## Development

```bash
cargo test -p astrid-hooks
```

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or [Apache 2.0](../../LICENSE-APACHE), at your option.
