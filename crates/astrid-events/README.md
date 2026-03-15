# astrid-events

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The IPC bus and message schemas.**

Every component in Astrid communicates through events. The kernel publishes lifecycle transitions. Capsules publish IPC messages. The CLI subscribes to streaming responses. The audit log observes everything. This crate provides the bus that carries those events and the payload schemas that give them structure.

Split by a `runtime` feature flag so WASM capsules can depend on schemas alone, without pulling in `tokio` or `chrono`.

## The bus

`EventBus` wraps `tokio::sync::broadcast` with a default capacity of 1,024 slots. All async subscribers receive every published event in order. Synchronous subscribers are notified via a `SubscriberRegistry` with `catch_unwind` isolation. One bad subscriber panics; the bus keeps running.

Clone-safe. All clones share the same broadcast channel and subscriber registry.

## Topic filtering

`bus.subscribe_topic(pattern)` yields only `AstridEvent::Ipc` messages where the topic matches the pattern. Supports exact matches, single-segment wildcards (`astrid.*.input`), and trailing wildcards (`astrid.v1.lifecycle.*`). Topic depth is capped at 20 segments.

## Lag tracking

`EventReceiver::drain_lagged()` returns the cumulative count of messages dropped due to channel overflow. Surface backpressure to callers without crashing the receiver.

## Rate limiting

`IpcRateLimiter` enforces per-source-ID quotas: 5 MB hard cap per payload, 10 MB/s rolling throughput limit per WASM guest. One instance per host process, shared via `Arc`.

## The event taxonomy

`AstridEvent` is a single enum covering:

- Agent lifecycle (runtime start/stop, session create/end)
- LLM request/stream lifecycle (request, stream deltas including `ReasoningDelta` for chain-of-thought, response, usage)
- Tool calls (execute request, result)
- IPC messages (topic-routed, with optional cryptographic signature)
- MCP events (server lifecycle, tool discovery)
- Sub-agent trees (spawn, complete)
- Capability and approval gates (request, decision)
- Budget tracking (reserve, commit, refund)
- Capsule loading (discovered, loaded, failed)
- Kernel/system events (shutdown, health)

Every variant carries `EventMetadata`: UUID, UTC timestamp, correlation/session/user IDs, and source component.

## IPC payloads

`IpcPayload` is always available (no `runtime` feature needed). It covers every host-capsule protocol message: `UserInput`, `AgentResponse`, `ApprovalRequired`/`ApprovalResponse`, `OnboardingRequired`, `LlmRequest`/`LlmStreamEvent`/`LlmResponse`, `ToolExecuteRequest`/`ToolExecuteResult`, `ElicitRequest`/`ElicitResponse`, `Connect`/`Disconnect`, `RawJson`, and `Custom`. Unknown tags deserialize to `IpcPayload::Unknown` instead of failing.

## Feature flags

| Feature | What it enables | Dependencies added |
|---|---|---|
| `runtime` (default) | `EventBus`, `EventReceiver`, `AstridEvent`, `IpcMessage`, `IpcRateLimiter`, subscriber registry | `tokio`, `chrono`, `dashmap`, `tracing`, `astrid-core` |
| (none) | `IpcPayload`, LLM types, `KernelRequest`/`KernelResponse` | `serde`, `serde_json`, `thiserror`, `uuid` only |

## Development

```bash
cargo test -p astrid-events
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
