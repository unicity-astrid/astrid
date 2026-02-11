//! Astralis Events - Event bus for the Astralis secure agent runtime SDK.
//!
//! This crate provides:
//! - Event types for all runtime operations
//! - Broadcast-based event bus for async subscribers
//! - Subscriber registry for synchronous handlers
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
//! use astralis_events::{EventBus, AstralisEvent, EventMetadata};
//!
//! # async fn example() {
//! // Create an event bus
//! let bus = EventBus::new();
//!
//! // Subscribe to events
//! let mut receiver = bus.subscribe();
//!
//! // Publish an event
//! bus.publish(AstralisEvent::RuntimeStarted {
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
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

mod bus;
mod event;
mod subscriber;

pub use bus::{DEFAULT_CHANNEL_CAPACITY, EventBus, EventReceiver};
pub use event::{AstralisEvent, EventMetadata};
pub use subscriber::{
    EventFilter, EventSubscriber, FilterSubscriber, SubscriberId, SubscriberRegistry,
};
