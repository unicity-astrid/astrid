# astrid-hooks

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**User-defined interceptors. Signal handlers for the OS.**

The kernel fires events at 23 points in the execution lifecycle. Hooks intercept those events and return typed verdicts: continue, block, ask the human, or continue with modifications. Shell commands, HTTP webhooks, and Extism WASM modules can all serve as handlers. No core engine changes required.

This is how operators customize Astrid without forking it. A security team blocks `rm` via a shell hook. A compliance system logs every tool call to an external webhook. A WASM module rewrites prompts before they reach the model. All without touching kernel code.

## How it works

A hook binds one handler to one event. When the kernel fires that event, the `HookExecutor` runs all matching hooks in priority order (lower integer runs first). Each handler returns a `HookResult`:

- **`Continue`** proceeds normally.
- **`Block { reason }`** short-circuits the chain and rejects the operation.
- **`Ask { question }`** pauses for human input.
- **`ContinueWith { modifications }`** proceeds with altered context.

If multiple hooks fire on the same event, `Block` takes absolute precedence. `Ask` takes precedence over `Continue`. `ContinueWith` modifications merge across hooks.

Each hook declares a `FailAction` for when the handler itself fails (timeout, crash, bad output): `Warn` (log and continue, the default), `Block` (treat failure as rejection), or `Ignore` (silent).

## 23 lifecycle events

Session start/end/reset. Prompt assembly. Tool calls (pre, post, error, result persist). Approval flows (pre, post). Context compaction (pre, post). Subagent start/stop. Model resolution. Message send/receive/sent. Agent loop end. Kernel start/stop. Notification.

## Three handler types

- **Command**: spawns a shell process, passes context as `ASTRID_HOOK_*` environment variables and JSON on stdin. Reads the verdict from stdout.
- **HTTP**: POSTs to a webhook URL. Reads the verdict from the response body.
- **WASM**: calls a function in an Extism module. Reads the verdict from the return value.

## TOML discovery

Hooks load from `HOOK.toml`, `hook.toml`, or `hooks.toml` files in `.astrid/hooks/` or configured extra paths. Example:

```toml
event = "pre_tool_call"
name = "block-rm"
timeout_secs = 5
fail_action = "block"

[handler]
type = "command"
command = "sh"
args = ["-c", "case \"$ASTRID_HOOK_DATA\" in *rm*) echo 'block: rm blocked';; *) echo continue;; esac"]
```

## Output protocol

Handlers signal results through stdout (command) or response body (HTTP):

- Empty or `"continue"`: `Continue`
- `"block: <reason>"`: `Block`
- `"ask: <question>"`: `Ask`
- JSON with `"action"` field: deserialized directly into `HookResult`

## Current state

The public API surface is intentionally narrow: `Hook`, `HookHandler`, `HookEvent`, and `HookResult`. The manager, executor, discovery, profiles, and handler modules are all `pub(crate)` internal, consumed by the kernel. Most builder methods on `Hook` and `HookHandler` are also `pub(crate)`. This crate defines the types and execution model. The kernel drives it.

Built-in profiles exist for `minimal`, `logging`, `security`, and `development` setups, but these are internal and not yet exposed as user-facing configuration.

## Development

```bash
cargo test -p astrid-hooks
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
