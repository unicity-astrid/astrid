//! Event bus for broadcasting events to subscribers.

use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, trace, warn};

use crate::event::AstridEvent;
use crate::subscriber::SubscriberRegistry;

/// Default channel capacity for the event bus.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Event bus for broadcasting events to all subscribers.
///
/// The event bus uses a broadcast channel to deliver events to all
/// connected receivers. Events are delivered asynchronously and in order.
#[derive(Debug)]
pub struct EventBus {
    /// Sender for broadcasting events.
    sender: broadcast::Sender<Arc<AstridEvent>>,
    /// Registry for synchronous subscribers.
    registry: SubscriberRegistry,
    /// Channel capacity.
    capacity: usize,
}

impl EventBus {
    /// Create a new event bus with default capacity.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new event bus with specified capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            registry: SubscriberRegistry::new(),
            capacity,
        }
    }

    /// Publish an event to all subscribers.
    ///
    /// This method broadcasts the event to all async subscribers and
    /// notifies all synchronous subscribers in the registry.
    ///
    /// Returns the number of async receivers that received the event.
    pub fn publish(&self, event: AstridEvent) -> usize {
        let event = Arc::new(event);

        trace!(event_type = %event.event_type(), "Publishing event");

        // Notify synchronous subscribers
        self.registry.notify(&event);

        // Broadcast to async subscribers
        if let Ok(count) = self.sender.send(Arc::clone(&event)) {
            debug!(
                event_type = %event.event_type(),
                receiver_count = count,
                "Event published"
            );
            count
        } else {
            // No receivers - this is fine
            trace!(event_type = %event.event_type(), "No receivers for event");
            0
        }
    }

    /// Subscribe to events.
    ///
    /// Returns a receiver that will receive all published events.
    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        EventReceiver {
            receiver: self.sender.subscribe(),
        }
    }

    /// Get the synchronous subscriber registry.
    #[must_use]
    pub fn registry(&self) -> &SubscriberRegistry {
        &self.registry
    }

    /// Get a mutable reference to the subscriber registry.
    pub fn registry_mut(&mut self) -> &mut SubscriberRegistry {
        &mut self.registry
    }

    /// Get the current number of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Get the channel capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        // Create a new bus that shares the same sender
        // but has its own registry
        Self {
            sender: self.sender.clone(),
            registry: SubscriberRegistry::new(),
            capacity: self.capacity,
        }
    }
}

/// Receiver for events from the event bus.
pub struct EventReceiver {
    receiver: broadcast::Receiver<Arc<AstridEvent>>,
}

impl EventReceiver {
    /// Receive the next event.
    ///
    /// Returns `None` if the channel is closed or if events were dropped
    /// due to the receiver being too slow.
    pub async fn recv(&mut self) -> Option<Arc<AstridEvent>> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    warn!(skipped = count, "Event receiver lagged, events dropped");
                    // Continue receiving
                },
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Try to receive the next event without blocking.
    ///
    /// Returns `Some(event)` if an event is available, or `None` if no event
    /// is available or the channel is closed.
    pub fn try_recv(&mut self) -> Option<Arc<AstridEvent>> {
        loop {
            match self.receiver.try_recv() {
                Ok(event) => return Some(event),
                Err(broadcast::error::TryRecvError::Lagged(count)) => {
                    warn!(skipped = count, "Event receiver lagged, events dropped");
                    // Continue receiving
                },
                Err(
                    broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed,
                ) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventMetadata;

    #[tokio::test]
    async fn test_event_bus_creation() {
        let bus = EventBus::new();
        assert_eq!(bus.capacity(), DEFAULT_CHANNEL_CAPACITY);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn test_event_bus_with_capacity() {
        let bus = EventBus::with_capacity(100);
        assert_eq!(bus.capacity(), 100);
    }

    #[tokio::test]
    async fn test_publish_and_receive() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();

        let event = AstridEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".to_string(),
        };

        let count = bus.publish(event);
        assert_eq!(count, 1);

        let msg = receiver.recv().await.unwrap();
        assert_eq!(msg.event_type(), "runtime_started");
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let mut receiver1 = bus.subscribe();
        let mut receiver2 = bus.subscribe();

        let event = AstridEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".to_string(),
        };

        let count = bus.publish(event);
        assert_eq!(count, 2);

        let obj1 = receiver1.recv().await.unwrap();
        let obj2 = receiver2.recv().await.unwrap();

        assert_eq!(obj1.event_type(), "runtime_started");
        assert_eq!(obj2.event_type(), "runtime_started");
    }

    #[tokio::test]
    async fn test_no_subscribers() {
        let bus = EventBus::new();

        let event = AstridEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".to_string(),
        };

        let count = bus.publish(event);
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_try_recv_empty() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();

        let result = receiver.try_recv();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_try_recv_with_event() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();

        let event = AstridEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".to_string(),
        };

        bus.publish(event);

        let result = receiver.try_recv();
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_subscriber_count() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);

        let receiver1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);

        let _receiver2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);

        drop(receiver1);
        // Note: subscriber count may not immediately reflect dropped receivers
    }
}
