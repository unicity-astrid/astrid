# astrid-hooks

[![Crates.io](https://img.shields.io/crates/v/astrid-hooks)](https://crates.io/crates/astrid-hooks)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

User-defined extension points and lifecycle hooks for the Astrid secure agent runtime.

The `astrid-hooks` crate provides a robust, event-driven extension architecture for the Astralis OS runtime. It enables developers and system administrators to intercept key execution phases—from session lifecycles and tool invocations to approval flows—and inject custom logic without modifying the core engine. By abstracting the execution medium into unified handlers, this crate allows Astralis to trigger shell commands, execute WebAssembly modules, dispatch HTTP webhooks, or cascade into LLM-based agent handlers.

## Core Features

- **Comprehensive Event Model**: Intercept execution at granular stages including `PreToolCall`, `PostToolCall`, `ApprovalRequest`, and `SessionStart`.
- **Pluggable Execution Handlers**: Dispatch events seamlessly to local shell environments, remote REST APIs, secure WebAssembly sandboxes, or secondary AI agents.
- **Context-Aware Execution**: Inject structured state and environment variables into handlers dynamically based on the triggering event.
- **Profile Management**: Group related hooks into named profiles (`HookProfile`) for environment-specific or agent-specific configurations.
- **Asynchronous Integration**: Fully async architecture built on `tokio` for high-throughput, non-blocking hook resolution.

## Architecture

Within the broader Astralis ecosystem, `astrid-hooks` sits between the event bus (`astrid-events`) and the core runtime scheduler. When the runtime reaches a defined transition point (e.g., about to execute a tool), it broadcasts an event. The `HookExecutor` matches the event against the registered `HookManager` routing table.

If a match is found, the executor materializes the specific `HookHandler`:
- **Command Handlers**: Spawn localized subprocesses, capturing `stdout`/`stderr` and returning the execution result. Useful for localized scripting and immediate side-effects.
- **HTTP Handlers**: Construct and dispatch RESTful payloads to external webhooks. Ideal for remote auditing, alerting, or decentralized processing.
- **WASM Handlers**: Sandbox logic execution using `extism` for secure, multi-tenant extension without OS-level permissions.
- **Agent Handlers**: Delegate complex, fuzzy logic to isolated subagents for recursive workflows.

## Quick Start

The fundamental workflow requires initializing a `HookManager`, defining your hooks, and executing them in context.

```rust
use astrid_hooks::{Hook, HookEvent, HookHandler, HookManager, HookExecutor};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut manager = HookManager::new();

    // Define a command-based hook to audit tool executions locally
    let audit_hook = Hook::new(HookEvent::PreToolCall)
        .with_handler(HookHandler::Command {
            command: "audit-logger".to_string(),
            args: vec!["--tool".to_string(), "$TOOL_NAME".to_string()],
            env: Default::default(),
        });
        
    // Define an HTTP webhook for real-time approval escalation
    let approval_hook = Hook::new(HookEvent::ApprovalRequest)
        .with_handler(HookHandler::Http {
            url: "https://internal.sec.corp/api/v1/approvals".to_string(),
            method: "POST".to_string(),
            headers: Default::default(),
        });

    manager.register(audit_hook);
    manager.register(approval_hook);
    
    // In practice, HookExecutor is integrated within the Astralis runtime
    // and invoked automatically during lifecycle transitions.
    let executor = HookExecutor::new(manager);
    
    Ok(())
}
```

### Supported Hook Events

The system currently tracks the following deterministic lifecycle states:
- `SessionStart` / `SessionEnd`
- `UserPrompt`
- `PreToolCall` / `PostToolCall` / `ToolCallError`
- `ApprovalRequest` / `ApprovalGranted` / `ApprovalDenied`
- `SubagentSpawn` / `SubagentComplete`

## Development

```bash
cargo test -p astrid-hooks
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
