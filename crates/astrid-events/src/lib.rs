//! Astrid Events - Event types and optional bus for the Astrid secure agent runtime.
//!
//! This crate provides:
//! - IPC payload types and LLM message schemas (always available, WASM-compatible)
//! - Broadcast-based event bus for async subscribers (requires `runtime` feature)
//! - Subscriber registry for synchronous handlers (requires `runtime` feature)
//!
//! # Feature Flags
//!
//! - `runtime` (default): Enables the event bus, subscriber registry, and full event
//!   types. Pulls in `tokio`, `chrono`, and other host-only dependencies. Not
//!   compatible with WASM targets.
//! - Without `runtime`: Only IPC payload schemas and LLM types are available. Suitable
//!   for WASM capsule crates that need to serialize/deserialize IPC messages.
//!
//! # Architecture
//!
//! Events are published to an `EventBus` which broadcasts them to all
//! subscribers. There are two ways to subscribe:
//!
//! 1. **Async receivers**: Use `bus.subscribe()` to get an `EventReceiver`
//!    that can be polled asynchronously.
//!
//! 2. **Synchronous subscribers**: Register implementations of `EventSubscriber`
//!    with the registry for immediate callback-based notification.
//!
//! # Example
//!
//! ```rust
//! use astrid_events::{EventBus, AstridEvent, EventMetadata};
//!
//! # async fn example() {
//! // Create an event bus
//! let bus = EventBus::new();
//!
//! // Subscribe to events
//! let mut receiver = bus.subscribe();
//!
//! // Publish an event
//! bus.publish(AstridEvent::RuntimeStarted {
//!     metadata: EventMetadata::new("runtime"),
//!     version: "0.1.0".to_string(),
//! });
//!
//! // Receive the event
//! let event = receiver.recv().await.unwrap();
//! assert_eq!(event.event_type(), "runtime_started");
//! # }
//! ```

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

#[cfg(feature = "runtime")]
pub mod prelude;

#[cfg(feature = "runtime")]
mod bus;
#[cfg(feature = "runtime")]
mod event;
pub mod ipc;
pub mod llm;
#[cfg(feature = "runtime")]
mod subscriber;

#[cfg(feature = "runtime")]
pub use bus::{DEFAULT_CHANNEL_CAPACITY, EventBus, EventReceiver};
#[cfg(feature = "runtime")]
pub use event::{AstridEvent, EventMetadata};
#[cfg(feature = "runtime")]
pub use ipc::IpcMessage;
pub use ipc::IpcPayload;
#[cfg(feature = "runtime")]
pub use ipc::IpcRateLimiter;
#[cfg(feature = "runtime")]
pub use subscriber::{
    EventFilter, EventSubscriber, FilterSubscriber, SubscriberId, SubscriberRegistry,
};
