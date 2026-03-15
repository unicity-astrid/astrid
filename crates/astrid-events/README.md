# astrid-events

[![Crates.io](https://img.shields.io/crates/v/astrid-events)](https://crates.io/crates/astrid-events)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Event bus and cross-boundary IPC schemas for the Astrid secure agent runtime.

`astrid-events` is the messaging backbone of Astrid. It provides a broadcast-based `EventBus` for distributing typed runtime events across all subscribers, a topic-routed IPC layer for communication between host and WASM capsules, and the shared payload schemas (`IpcPayload`, LLM types) that capsules use without pulling in host-only dependencies. The crate is split by a `runtime` feature flag so WASM capsules can depend on only the schema types without importing `tokio` or `chrono`.

## Core Features

- **Typed event taxonomy**: A single `AstridEvent` enum covers agent lifecycle, session management, LLM request/stream lifecycle, tool calls, MCP server events, sub-agent tree events, capability and approval gates, budget tracking, capsule loading, and kernel/system events.
- **Broadcast event bus**: `EventBus` wraps a `tokio::sync::broadcast` channel with a default capacity of 1,024 slots. All async subscribers receive every published event in order.
- **Topic-filtered IPC subscriptions**: `bus.subscribe_topic(pattern)` yields only `AstridEvent::Ipc` messages whose topic matches. Supports exact matches (`astrid.cli.input`), single-segment wildcards (`astrid.*.input`), and trailing wildcards (`astrid.v1.lifecycle.*`) that match one or more remaining segments up to a depth of 20.
- **Lag tracking**: `EventReceiver::drain_lagged()` returns and resets the cumulative count of messages dropped due to channel overflow, so callers can surface backpressure without crashing.
- **Synchronous subscriber registry**: `SubscriberRegistry` dispatches events to sync callbacks inside `publish()`. Arc-based copy-on-write map makes `unregister` safe to call from within an `on_event` handler without deadlocking.
- **Panic isolation**: Panics in synchronous subscribers are caught with `std::panic::catch_unwind` so one bad subscriber cannot take down the bus.
- **Structured IPC payloads**: `IpcPayload` covers every host-capsule protocol message - `UserInput`, `AgentResponse`, `ApprovalRequired`/`ApprovalResponse`, `OnboardingRequired` with typed `OnboardingField` descriptors, `LlmRequest`/`LlmStreamEvent`/`LlmResponse`, `ToolExecuteRequest`/`ToolExecuteResult`, `SelectionRequired`, `ElicitRequest`/`ElicitResponse`, `Connect`/`Disconnect`, and a `Custom` escape hatch. Unknown type tags deserialize to `IpcPayload::Unknown` instead of erroring.
- **IPC rate limiter**: `IpcRateLimiter` is a per-source token-bucket that enforces a 5 MB hard payload cap and a 10 MB/s rolling throughput limit per WASM guest.
- **LLM schemas**: The `llm` module defines `Message`, `MessageRole`, `MessageContent`, `ToolCall`, `ToolCallResult`, `LlmToolDefinition`, `StreamEvent` (including `ReasoningDelta` for chain-of-thought models), `LlmResponse`, `StopReason`, and `Usage` - all WASM-compatible.
- **Kernel management API**: `KernelRequest` and `KernelResponse` define the management protocol between the CLI and the background daemon (`InstallCapsule`, `ListCapsules`, `GetCommands`, `GetCapsuleMetadata`, `ApproveCapability`).
- **WASM-compatible subset**: Without the `runtime` feature, `IpcPayload`, all LLM types, and `KernelRequest`/`KernelResponse` are still available. No `tokio`, no `chrono`, no `dashmap`.

## Quick Start

Add to `Cargo.toml`:

```toml
# Full runtime (host-side, tokio required)
[dependencies]
astrid-events = { version = "0.2", features = ["runtime"] }

# Schema-only (WASM capsules)
[dependencies]
astrid-events = { version = "0.2", default-features = false }
```

## API Reference

### Key Types

#### `EventBus` (requires `runtime`)

The central hub. Clone it freely - all clones share the same broadcast channel and subscriber registry.

```rust
use astrid_events::prelude::*;

let bus = EventBus::new();                          // capacity 1024
let bus2 = EventBus::with_capacity(256);

bus.publish(AstridEvent::RuntimeStarted {
    metadata: EventMetadata::new("daemon"),
    version: "0.2.0".to_string(),
});

let count = bus.subscriber_count();
let cap   = bus.capacity();
```

#### `EventReceiver` (requires `runtime`)

Returned by `bus.subscribe()` and `bus.subscribe_topic()`. Not `Clone` - each call creates an independent receiver.

```rust
let mut rx = bus.subscribe();

// Blocking async receive
if let Some(event) = rx.recv().await {
    println!("{}", event.event_type()); // "astrid.v1.lifecycle.runtime_started"
}

// Non-blocking poll
if let Some(event) = rx.try_recv() { /* ... */ }

// Check for dropped messages since last call
let dropped = rx.drain_lagged();
```

Topic subscriptions filter to `AstridEvent::Ipc` only - non-IPC events are invisible to a topic receiver:

```rust
let mut ipc_rx = bus.subscribe_topic("astrid.v1.request.*");
```

#### `AstridEvent` (requires `runtime`)

All variants carry an `EventMetadata` (UUID, UTC timestamp, optional correlation/session/user IDs, source string) plus variant-specific fields. Call `.event_type()` for the `astrid.v1.*` string tag, or `.metadata()` for the shared header.

Categories and their `event_type` prefixes:

| Category | Variants |
|---|---|
| Lifecycle | `RuntimeStarted`, `RuntimeStopped`, `AgentStarted`, `AgentStopped` |
| Session | `SessionCreated`, `SessionEnded`, `SessionResumed` |
| Cognitive loop | `PromptBuilding`, `MessageSending`, `ContextCompactionStarted/Completed`, `SessionResetting`, `ModelResolving`, `AgentLoopCompleted`, `ToolResultPersisting` |
| Message flow | `MessageReceived`, `MessageSent`, `MessageProcessed` |
| LLM | `LlmRequestStarted/Completed`, `LlmStreamStarted/Chunk/Completed` |
| Tools | `ToolCallStarted`, `ToolCallCompleted`, `ToolCallFailed` |
| MCP | `McpServerConnected/Disconnected`, `McpToolCalled/Completed` |
| Sub-agents | `SubAgentSpawned/Progress/Completed/Failed/Cancelled` |
| Security | `CapabilityGranted/Revoked/Checked`, `AuthorizationDenied`, `SecurityViolation` |
| Approvals | `ApprovalRequested/Granted/Denied` |
| Budget | `BudgetAllocated`, `BudgetWarning`, `BudgetExceeded` |
| Capsules | `CapsuleLoaded/Failed/Unloaded` |
| System | `KernelStarted/Shutdown`, `ConfigReloaded/Changed`, `HealthCheckCompleted` |
| Audit | `AuditEntryCreated` |
| IPC | `Ipc { message: IpcMessage }` |
| Custom | `Custom { name, data: Value }` |

#### `EventMetadata`

Builder-style constructor:

```rust
let meta = EventMetadata::new("my-service")
    .with_correlation_id(correlation_uuid)
    .with_session_id(session_uuid)
    .with_user_id(user_uuid);
```

#### `IpcPayload` (always available)

```rust
use astrid_events::IpcPayload;

let payload = IpcPayload::UserInput {
    text: "deploy staging".into(),
    session_id: "default".into(),
    context: None,
};

// Serialize for a WASM guest (strips outer IpcMessage envelope)
let bytes = payload.to_guest_bytes()?;

// Deserialize from guest JSON, falling back to Custom on unknown tags
let payload = IpcPayload::from_json_value(json_value);

// Check whether a raw type tag string is a known protocol variant
assert!(IpcPayload::is_known_tag("user_input"));
```

#### `IpcMessage` (requires `runtime`)

Wraps an `IpcPayload` with a topic string, `source_id` UUID, UTC timestamp, and an optional cryptographic signature byte vector:

```rust
use astrid_events::{IpcMessage, IpcPayload};
use uuid::Uuid;

let msg = IpcMessage::new(
    "astrid.cli.input",
    IpcPayload::UserInput { text: "hi".into(), session_id: "s1".into(), context: None },
    Uuid::new_v4(),
).with_signature(signature_bytes);
```

#### `IpcRateLimiter` (requires `runtime`)

One instance per host process, shared across capsules via `Arc`:

```rust
use astrid_events::IpcRateLimiter;
use uuid::Uuid;

let limiter = IpcRateLimiter::new();

match limiter.check_quota(source_id, payload_bytes.len()) {
    Ok(())   => { /* publish */ }
    Err(msg) => { /* reject: "Payload too large" or "Rate limit exceeded" */ }
}
```

Hard limits enforced per source UUID: 5 MB per message, 10 MB per second.

#### LLM types (always available, in `astrid_events::llm`)

`Message`, `MessageRole`, `MessageContent`, `ContentPart`, `ToolCall`, `ToolCallResult`, `LlmToolDefinition`, `StreamEvent`, `LlmResponse`, `StopReason`, `Usage`.

```rust
use astrid_events::llm::{Message, ToolCall, ToolCallResult};

let user_msg   = Message::user("Summarise this file.");
let tool_call  = ToolCall::new("c1", "filesystem:read_file")
                     .with_arguments(serde_json::json!({"path": "/etc/hosts"}));
let result     = ToolCallResult::success("c1", "127.0.0.1 localhost");
```

`StreamEvent` includes `ReasoningDelta` for chain-of-thought models (DeepSeek, OpenAI o-series, etc.).

#### Kernel API types (always available, in `astrid_events::kernel_api`)

`KernelRequest`, `KernelResponse`, `CapsuleMetadataEntry`, `LlmProviderInfo`, `CommandInfo`.

The well-known system session UUID is exposed as `astrid_events::kernel_api::SYSTEM_SESSION_UUID`.

#### Onboarding types (always available, in `astrid_events::ipc`)

`OnboardingField` and `OnboardingFieldType` (`Text`, `Secret`, `Enum(Vec<String>)`, `Array`) describe typed env-var prompts used during capsule installation. `SelectionOption` supports generic picker UIs.

### Prelude

```rust
use astrid_events::prelude::*;
// Re-exports: EventBus, EventReceiver, AstridEvent, EventMetadata
```

## Feature Flags

| Feature | Default | Description |
|---|---|---|
| `runtime` | yes | Enables `EventBus`, `EventReceiver`, `AstridEvent`, `IpcMessage`, `IpcRateLimiter`. Pulls in `tokio`, `chrono`, `dashmap`, `tracing`. Not compatible with `wasm32-unknown-unknown`. |
| (none) | - | Only `IpcPayload`, LLM types, and kernel API types are compiled. Safe for WASM capsule crates. |

## Development

```bash
cargo test -p astrid-events
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
