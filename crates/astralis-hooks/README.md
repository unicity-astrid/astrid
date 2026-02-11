# astralis-hooks

User-defined extension points for the Astralis runtime.

This crate provides a flexible hook system that allows users to extend the behavior of the Astralis runtime at key points in the execution flow.

## Hook Events

Hooks can be triggered on various events:

| Event | Description |
|-------|-------------|
| `SessionStart` | When a session begins |
| `SessionEnd` | When a session ends |
| `UserPrompt` | When a user submits a prompt |
| `PreToolCall` | Before a tool is invoked |
| `PostToolCall` | After a tool completes successfully |
| `ToolCallError` | When a tool call fails |
| `ApprovalRequest` | When approval is requested |
| `ApprovalGranted` | When approval is granted |
| `ApprovalDenied` | When approval is denied |
| `SubagentSpawn` | When a subagent is created |
| `SubagentComplete` | When a subagent finishes |

## Handler Types

| Handler | Description | Status |
|---------|-------------|--------|
| **Command** | Execute shell commands | Available |
| **HTTP** | Call webhooks | Available |
| **WASM** | Run WebAssembly modules | Phase 3 |
| **Agent** | Invoke LLM-based handlers | Phase 3 |

## Usage

```rust
use astralis_hooks::{Hook, HookEvent, HookHandler, HookManager};

// Create a hook manager
let mut manager = HookManager::new();

// Register a command hook for tool calls
let hook = Hook::new(HookEvent::PreToolCall)
    .with_handler(HookHandler::Command {
        command: "echo".to_string(),
        args: vec!["Tool called: $TOOL_NAME".to_string()],
        env: Default::default(),
    });

manager.register(hook);

// Register an HTTP webhook for approvals
let webhook = Hook::new(HookEvent::ApprovalRequest)
    .with_handler(HookHandler::Http {
        url: "https://example.com/webhook".to_string(),
        method: "POST".to_string(),
        headers: Default::default(),
    });

manager.register(webhook);
```

## Key Exports

- `Hook` - A single hook definition
- `HookEvent` - Events that trigger hooks
- `HookHandler` - Handler implementations (Command, HTTP, WASM, Agent)
- `HookManager` - Manages hook registration and lookup
- `HookExecutor` - Executes hooks with context
- `HookProfile` - Named collections of hooks

## License

This crate is licensed under the MIT license.
