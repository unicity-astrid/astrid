# astrid-events

Event bus for the Astrid secure agent runtime.

## Overview

This crate provides event types and a broadcast-based event bus for
communicating runtime operations across the Astrid system.

## Features

- **Event Types**: Comprehensive event types for all runtime operations
- **Broadcast Event Bus**: Async event distribution to multiple subscribers
- **Dual Subscription Modes**:
  - Async receivers via `bus.subscribe()` for polling
  - Synchronous handlers via `EventSubscriber` trait for callbacks
- **Event Filtering**: Filter events by type with `FilterSubscriber`
- **Subscriber Registry**: Manage synchronous handler registrations

## Usage

### Async Subscription

```rust
use astrid_events::{EventBus, AstridEvent, EventMetadata};

async fn example() {
    // Create an event bus
    let bus = EventBus::new();

    // Subscribe to events
    let mut receiver = bus.subscribe();

    // Publish an event
    bus.publish(AstridEvent::RuntimeStarted {
        metadata: EventMetadata::new("runtime"),
        version: "0.1.0".to_string(),
    });

    // Receive the event
    let event = receiver.recv().await.unwrap();
    assert_eq!(event.event_type(), "runtime_started");
}
```

### Synchronous Subscription

```rust
use astrid_events::{EventSubscriber, SubscriberRegistry, AstridEvent};

struct MySubscriber;

impl EventSubscriber for MySubscriber {
    fn on_event(&self, event: &AstridEvent) {
        println!("Received: {}", event.event_type());
    }
}

let registry = SubscriberRegistry::new();
registry.register(Box::new(MySubscriber));
```

## Key Exports

| Export | Description |
|--------|-------------|
| `EventBus` | Broadcast-based event distribution |
| `EventReceiver` | Async receiver for subscribed events |
| `AstridEvent` | Enum of all event types |
| `EventMetadata` | Common metadata for events |
| `EventSubscriber` | Trait for synchronous handlers |
| `SubscriberRegistry` | Registry for synchronous subscribers |

## License

This crate is licensed under the MIT license.
