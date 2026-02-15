//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_events::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust
//! use astrid_events::prelude::*;
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

// Event bus
pub use crate::{DEFAULT_CHANNEL_CAPACITY, EventBus, EventReceiver};

// Events
pub use crate::{AstridEvent, EventMetadata};

// Subscriber system
pub use crate::{EventFilter, EventSubscriber, FilterSubscriber, SubscriberId, SubscriberRegistry};
