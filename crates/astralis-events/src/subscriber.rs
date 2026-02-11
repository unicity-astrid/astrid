//! Event subscriber trait and registry.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::event::AstralisEvent;

/// Filter function type for event subscribers.
pub type EventFilter = Box<dyn Fn(&AstralisEvent) -> bool + Send + Sync>;

/// Trait for synchronous event subscribers.
///
/// Implement this trait to receive events synchronously. Note that
/// subscribers should not perform heavy work in the `on_event` method
/// as it blocks the event bus.
pub trait EventSubscriber: Send + Sync {
    /// Called when an event is published.
    ///
    /// This method should return quickly. For heavy processing,
    /// consider using async subscribers via `EventReceiver` instead.
    fn on_event(&self, event: &AstralisEvent);

    /// Optional filter for event types.
    ///
    /// Return `true` to receive the event, `false` to skip it.
    /// Default implementation accepts all events.
    fn accepts(&self, event: &AstralisEvent) -> bool {
        let _ = event;
        true
    }

    /// Optional name for debugging.
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "anonymous"
    }
}

/// Registration handle for a subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriberId(Uuid);

impl SubscriberId {
    /// Create a new subscriber ID.
    #[must_use]
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Registry for managing synchronous event subscribers.
#[derive(Default)]
pub struct SubscriberRegistry {
    subscribers: RwLock<HashMap<SubscriberId, Arc<dyn EventSubscriber>>>,
}

impl std::fmt::Debug for SubscriberRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.subscribers.read().map(|s| s.len()).unwrap_or_default();
        f.debug_struct("SubscriberRegistry")
            .field("subscriber_count", &count)
            .finish()
    }
}

impl SubscriberRegistry {
    /// Create a new subscriber registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a subscriber.
    ///
    /// Returns a handle that can be used to unregister the subscriber.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn register(&self, subscriber: Arc<dyn EventSubscriber>) -> SubscriberId {
        let id = SubscriberId::new();
        let name = subscriber.name().to_string();

        let mut subs = self.subscribers.write().expect("lock poisoned");
        subs.insert(id, subscriber);

        debug!(subscriber_name = %name, "Subscriber registered");
        id
    }

    /// Unregister a subscriber.
    ///
    /// Returns `true` if the subscriber was found and removed.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn unregister(&self, id: SubscriberId) -> bool {
        let mut subs = self.subscribers.write().expect("lock poisoned");
        let removed = subs.remove(&id).is_some();

        if removed {
            debug!("Subscriber unregistered");
        }

        removed
    }

    /// Notify all subscribers of an event.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn notify(&self, event: &AstralisEvent) {
        let subs = self.subscribers.read().expect("lock poisoned");

        for (id, subscriber) in subs.iter() {
            if subscriber.accepts(event) {
                trace!(
                    subscriber_name = %subscriber.name(),
                    event_type = %event.event_type(),
                    "Notifying subscriber"
                );

                // Catch panics to prevent one subscriber from affecting others
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    subscriber.on_event(event);
                }));

                if let Err(e) = result {
                    warn!(
                        subscriber_id = ?id,
                        subscriber_name = %subscriber.name(),
                        error = ?e,
                        "Subscriber panicked"
                    );
                }
            }
        }
    }

    /// Get the number of registered subscribers.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.subscribers.read().expect("lock poisoned").len()
    }

    /// Check if the registry is empty.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.subscribers.read().expect("lock poisoned").is_empty()
    }

    /// Clear all subscribers.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn clear(&self) {
        let mut subs = self.subscribers.write().expect("lock poisoned");
        subs.clear();
        debug!("All subscribers cleared");
    }
}

/// A simple filter-based subscriber.
pub struct FilterSubscriber<F>
where
    F: Fn(&AstralisEvent) + Send + Sync,
{
    name: String,
    filter: Option<EventFilter>,
    handler: F,
}

impl<F> FilterSubscriber<F>
where
    F: Fn(&AstralisEvent) + Send + Sync,
{
    /// Create a new filter subscriber.
    pub fn new(name: impl Into<String>, handler: F) -> Self {
        Self {
            name: name.into(),
            filter: None,
            handler,
        }
    }

    /// Add a filter to this subscriber.
    #[must_use]
    pub fn with_filter<P>(mut self, predicate: P) -> Self
    where
        P: Fn(&AstralisEvent) -> bool + Send + Sync + 'static,
    {
        self.filter = Some(Box::new(predicate));
        self
    }
}

impl<F> EventSubscriber for FilterSubscriber<F>
where
    F: Fn(&AstralisEvent) + Send + Sync,
{
    fn on_event(&self, event: &AstralisEvent) {
        (self.handler)(event);
    }

    fn accepts(&self, event: &AstralisEvent) -> bool {
        match &self.filter {
            Some(f) => f(event),
            None => true,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventMetadata;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSubscriber {
        name: String,
        count: AtomicUsize,
    }

    impl CountingSubscriber {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                count: AtomicUsize::new(0),
            }
        }

        fn count(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }
    }

    impl EventSubscriber for CountingSubscriber {
        fn on_event(&self, _event: &AstralisEvent) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_registry_register_unregister() {
        let registry = SubscriberRegistry::new();
        assert!(registry.is_empty());

        let subscriber = Arc::new(CountingSubscriber::new("test"));
        let id = registry.register(subscriber);

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());

        let removed = registry.unregister(id);
        assert!(removed);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_registry_notify() {
        let registry = SubscriberRegistry::new();
        let subscriber = Arc::new(CountingSubscriber::new("test"));
        registry.register(Arc::clone(&subscriber) as Arc<dyn EventSubscriber>);

        let event = AstralisEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".to_string(),
        };

        registry.notify(&event);
        assert_eq!(subscriber.count(), 1);

        registry.notify(&event);
        assert_eq!(subscriber.count(), 2);
    }

    #[test]
    fn test_registry_multiple_subscribers() {
        let registry = SubscriberRegistry::new();
        let sub1 = Arc::new(CountingSubscriber::new("sub1"));
        let sub2 = Arc::new(CountingSubscriber::new("sub2"));

        registry.register(Arc::clone(&sub1) as Arc<dyn EventSubscriber>);
        registry.register(Arc::clone(&sub2) as Arc<dyn EventSubscriber>);

        let event = AstralisEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".to_string(),
        };

        registry.notify(&event);

        assert_eq!(sub1.count(), 1);
        assert_eq!(sub2.count(), 1);
    }

    #[test]
    fn test_filter_subscriber() {
        let received = Arc::new(AtomicUsize::new(0));
        let received_clone = Arc::clone(&received);

        let subscriber = FilterSubscriber::new("security_only", move |_event| {
            received_clone.fetch_add(1, Ordering::SeqCst);
        })
        .with_filter(|e| e.is_security_event());

        let registry = SubscriberRegistry::new();
        registry.register(Arc::new(subscriber));

        // Non-security event should be filtered
        let event1 = AstralisEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".to_string(),
        };
        registry.notify(&event1);
        assert_eq!(received.load(Ordering::SeqCst), 0);

        // Security event should be received
        let event2 = AstralisEvent::CapabilityGranted {
            metadata: EventMetadata::new("test"),
            capability_id: Uuid::new_v4(),
            resource: "test".to_string(),
            action: "execute".to_string(),
        };
        registry.notify(&event2);
        assert_eq!(received.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_registry_clear() {
        let registry = SubscriberRegistry::new();

        let sub1 = Arc::new(CountingSubscriber::new("sub1"));
        let sub2 = Arc::new(CountingSubscriber::new("sub2"));

        registry.register(sub1);
        registry.register(sub2);

        assert_eq!(registry.len(), 2);

        registry.clear();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_unregister_nonexistent() {
        let registry = SubscriberRegistry::new();
        let fake_id = SubscriberId::new();

        let removed = registry.unregister(fake_id);
        assert!(!removed);
    }
}
