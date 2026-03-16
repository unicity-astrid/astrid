//! IPC rate limiter for publish events.

use uuid::Uuid;

/// Simple token-bucket rate limiter for IPC publish events.
#[derive(Debug)]
pub struct IpcRateLimiter {
    state: dashmap::DashMap<Uuid, (std::time::Instant, usize)>,
    last_prune: std::sync::Mutex<std::time::Instant>,
}

impl IpcRateLimiter {
    /// Create a new IPC rate limiter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: dashmap::DashMap::new(),
            last_prune: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }

    /// Check if a plugin (`source_id`) is allowed to publish a payload of `size_bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error string if rate-limited or if the payload is too large.
    #[expect(clippy::collapsible_if)]
    pub fn check_quota(&self, source_id: Uuid, size_bytes: usize) -> Result<(), String> {
        // Hard limit on payload size to prevent OOM
        if size_bytes > 5 * 1024 * 1024 {
            return Err("Payload too large".to_string());
        }

        let now = std::time::Instant::now();

        // Lazy prune stale entries to prevent memory leaks when shared globally.
        if self.state.len() > 1000 {
            if let Ok(mut last) = self.last_prune.try_lock() {
                if now.saturating_duration_since(*last).as_secs() > 60 {
                    *last = now;
                    self.state
                        .retain(|_, v| now.saturating_duration_since(v.0).as_secs() < 1);
                }
            }
        }

        let mut entry = self.state.entry(source_id).or_insert((now, 0));

        // Reset window if more than 1 second has passed
        if now.saturating_duration_since(entry.0).as_secs() >= 1 {
            entry.0 = now;
            entry.1 = 0;
        }

        // Hard limit on total bytes per second (10MB)
        if entry.1.saturating_add(size_bytes) > 10 * 1024 * 1024 {
            return Err("Rate limit exceeded".to_string());
        }

        entry.1 = entry.1.saturating_add(size_bytes);

        Ok(())
    }
}

impl Default for IpcRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
