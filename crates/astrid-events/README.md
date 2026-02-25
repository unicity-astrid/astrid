# astrid-events

[![Crates.io](https://img.shields.io/crates/v/astrid-events)](https://crates.io/crates/astrid-events)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Asynchronous event bus and cross-boundary IPC router for the Astralis OS runtime.

`astrid-events` acts as the central nervous system connecting the isolated components of the Astralis OS. It provides a robust, broadcast-based distribution layer that shuttles internal host service events and strongly typed IPC messages across WASM boundaries. By combining high-performance asynchronous polling with strictly regulated topic matching, this crate ensures seamless and predictable communication without risking host stability.

## Core Features

- **Dual-Mode Subscriptions**: Subscribe via async streams (`bus.subscribe()`) or synchronous callbacks (`EventSubscriber`).
- **Topic-Based Routing**: Filter cross-boundary IPC messages by topic string. Supports exact matches (`astrid.cli.input`) and trailing wildcards (`astrid.*`).
- **Cross-Boundary IPC Schemas**: Standardized, strongly typed payloads (`UserInput`, `AgentResponse`, `ApprovalRequired`) for safe communication with untrusted WASM guests.
- **Swarm Signatures**: Built-in support for cryptographic signatures on IPC messages, enabling stateless verification across distributed Astralis nodes.
- **Quota Enforcement**: Integrated token-bucket rate limiter (`IpcRateLimiter`) that strictly caps guest payload sizes (5MB hard limit) and transmission frequencies (10MB/s).
- **Memory-Safe Registry**: Advanced subscriber registry explicitly designed to prevent `Arc` reference cycles when dealing with self-referential callbacks.

## Architecture

The crate is built around the `EventBus`, a concurrent broadcast channel that guarantees ordered, asynchronous event delivery. Every published `AstridEvent` is instantly available to all connected subscribers.

To accommodate the diverse execution contexts within Astralis (e.g., background daemon tasks vs. immediate audit loggers), the bus supports a dual-mode subscription model:
1. **Asynchronous Receivers**: Yields an `EventReceiver` stream for polling loops.
2. **Synchronous Subscribers**: Registers `EventSubscriber` traits via the `SubscriberRegistry` for immediate, inline callback execution.

## Quick Start

### Asynchronous Subscriptions

The primary method for background tasks is to obtain an `EventReceiver` and poll it asynchronously.

```rust
use astrid_events::{EventBus, AstridEvent, EventMetadata};

async fn monitor_runtime() {
    let bus = EventBus::new();
    let mut receiver = bus.subscribe();

    // Publish an internal system event
    bus.publish(AstridEvent::RuntimeStarted {
        metadata: EventMetadata::new("system_monitor"),
        version: "1.0.0".to_string(),
    });

    // Await the next event in the queue
    while let Some(event) = receiver.recv().await {
        println!("Received event: {}", event.event_type());
    }
}
```

### IPC Topic Routing

When bridging communication from WASM guests or remote agents, subscribe exclusively to specific IPC topics to filter out internal system noise.

```rust
use astrid_events::EventBus;

async fn handle_cli_input(bus: &EventBus) {
    // Subscribe using a trailing wildcard to capture all CLI-related IPC messages
    let mut cli_receiver = bus.subscribe_topic("astrid.cli.*");

    while let Some(event) = cli_receiver.recv().await {
        if let astrid_events::AstridEvent::Ipc { message, .. } = &*event {
            println!("Intercepted IPC on topic {}: {:?}", message.topic, message.payload);
        }
    }
}
```

### Synchronous Callbacks

For components requiring immediate notification without the overhead of async polling (provided they do not block), use the synchronous registry.

```rust
use std::sync::Arc;
use astrid_events::{EventBus, EventSubscriber, AstridEvent, FilterSubscriber};

fn setup_sync_logger(bus: &EventBus) {
    let subscriber = FilterSubscriber::new("audit_logger", |event| {
        println!("Synchronous audit log: {}", event.event_type());
    })
    // Restrict the callback to specific event types
    .with_filter(AstridEvent::is_security_event);

    bus.registry().register(Arc::new(subscriber));
}
```

> **Warning:** Storing a strong clone of `EventBus` inside a synchronous subscriber will create an `Arc` reference cycle, resulting in a memory leak. Use `std::sync::Weak<EventBus>` if the subscriber must publish subsequent events, or utilize the `bus` reference provided directly in the `on_event` signature.

## Rate Limiting

To protect the host process from aggressive WASM guests or misconfigured plugins, `astrid-events` enforces throughput quotas via the `IpcRateLimiter`.

```rust
use astrid_events::{IpcRateLimiter, QuotaError};
use uuid::Uuid;

let limiter = IpcRateLimiter::new();
let plugin_id = Uuid::new_v4();
let payload_size_bytes = 1024 * 500; // 500 KB

match limiter.check_quota(plugin_id, payload_size_bytes) {
    Ok(_) => println!("Payload accepted"),
    Err(QuotaError::RateLimited) => println!("Plugin exceeded 10MB/s frequency limit"),
    Err(QuotaError::PayloadTooLarge) => println!("Payload exceeded 5MB hard limit"),
}
```

## Development

```bash
cargo test -p astrid-events
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
