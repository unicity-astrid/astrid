//! Rate limiting for MCP operations.
//!
//! Provides rate limiting for:
//! - Elicitation requests per server
//! - Sampling requests per server
//! - Global pending request limits

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for a rate limit.
#[derive(Debug, Clone, Copy)]
pub struct RateLimit {
    /// Maximum number of requests allowed in the window.
    pub max_requests: u32,
    /// Time window for the limit.
    pub window: Duration,
}

impl RateLimit {
    /// Create a new rate limit.
    #[must_use]
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
        }
    }

    /// Create a limit of N requests per second.
    #[must_use]
    pub fn per_second(requests: u32) -> Self {
        Self::new(requests, Duration::seconds(1))
    }

    /// Create a limit of N requests per minute.
    #[must_use]
    pub fn per_minute(requests: u32) -> Self {
        Self::new(requests, Duration::minutes(1))
    }

    /// Create a limit of N requests per hour.
    #[must_use]
    pub fn per_hour(requests: u32) -> Self {
        Self::new(requests, Duration::hours(1))
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::per_minute(60)
    }
}

/// Rate limits configuration.
#[derive(Debug, Clone)]
pub struct RateLimits {
    /// Rate limit for elicitation requests per server.
    pub elicitation_per_server: RateLimit,
    /// Rate limit for sampling requests per server.
    pub sampling_per_server: RateLimit,
    /// Maximum number of pending requests globally.
    pub global_pending_requests: u32,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            elicitation_per_server: RateLimit::per_minute(10),
            sampling_per_server: RateLimit::per_minute(30),
            global_pending_requests: 100,
        }
    }
}

impl RateLimits {
    /// Create rate limits with custom values.
    #[must_use]
    pub fn new(
        elicitation_per_server: RateLimit,
        sampling_per_server: RateLimit,
        global_pending_requests: u32,
    ) -> Self {
        Self {
            elicitation_per_server,
            sampling_per_server,
            global_pending_requests,
        }
    }

    /// Create permissive limits (for testing or trusted environments).
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            elicitation_per_server: RateLimit::per_second(100),
            sampling_per_server: RateLimit::per_second(100),
            global_pending_requests: 1000,
        }
    }

    /// Create strict limits.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            elicitation_per_server: RateLimit::per_minute(5),
            sampling_per_server: RateLimit::per_minute(10),
            global_pending_requests: 20,
        }
    }
}

/// Tracks requests in a sliding window.
#[derive(Debug)]
struct WindowTracker {
    /// Timestamps of requests in the current window.
    requests: Vec<DateTime<Utc>>,
    /// The rate limit configuration.
    limit: RateLimit,
}

impl WindowTracker {
    fn new(limit: RateLimit) -> Self {
        Self {
            requests: Vec::new(),
            limit,
        }
    }

    /// Try to record a request. Returns true if allowed, false if rate limited.
    fn try_request(&mut self) -> bool {
        let now = Utc::now();
        let window_start = now - self.limit.window;

        // Remove requests outside the window
        self.requests.retain(|t| *t > window_start);

        // Check if we're at the limit
        if self.requests.len() >= self.limit.max_requests as usize {
            return false;
        }

        // Record the request
        self.requests.push(now);
        true
    }

    /// Get the number of requests in the current window.
    fn current_count(&self) -> usize {
        let now = Utc::now();
        let window_start = now - self.limit.window;
        self.requests.iter().filter(|t| **t > window_start).count()
    }

    /// Get remaining requests in the current window.
    #[allow(clippy::cast_possible_truncation)]
    fn remaining(&self) -> u32 {
        let count = self.current_count();
        self.limit.max_requests.saturating_sub(count as u32)
    }

    /// Get when the next request will be allowed (if rate limited).
    fn retry_after(&self) -> Option<Duration> {
        if self.remaining() > 0 {
            return None;
        }

        let now = Utc::now();
        let window_start = now - self.limit.window;

        // Find the oldest request in the window
        self.requests
            .iter()
            .filter(|t| **t > window_start)
            .min()
            .map(|oldest| (*oldest + self.limit.window) - now)
    }
}

/// Result of a rate limit check.
#[derive(Debug, Clone)]
pub enum RateLimitResult {
    /// Request is allowed.
    Allowed {
        /// Remaining requests in the window.
        remaining: u32,
    },
    /// Request is denied due to rate limiting.
    Denied {
        /// Time until next request is allowed.
        retry_after: Duration,
    },
}

impl RateLimitResult {
    /// Check if the request is allowed.
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// Get the retry-after duration if denied.
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Allowed { .. } => None,
            Self::Denied { retry_after } => Some(*retry_after),
        }
    }
}

/// Rate limiter for MCP operations.
#[derive(Debug)]
pub struct RateLimiter {
    /// Configuration.
    limits: RateLimits,
    /// Elicitation trackers per server.
    elicitation: Arc<RwLock<HashMap<String, WindowTracker>>>,
    /// Sampling trackers per server.
    sampling: Arc<RwLock<HashMap<String, WindowTracker>>>,
    /// Current pending request count.
    pending_count: Arc<RwLock<u32>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(RateLimits::default())
    }
}

impl RateLimiter {
    /// Create a new rate limiter with the given limits.
    #[must_use]
    pub fn new(limits: RateLimits) -> Self {
        Self {
            limits,
            elicitation: Arc::new(RwLock::new(HashMap::new())),
            sampling: Arc::new(RwLock::new(HashMap::new())),
            pending_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Check and record an elicitation request.
    pub async fn check_elicitation(&self, server: &str) -> RateLimitResult {
        let mut trackers = self.elicitation.write().await;
        let tracker = trackers
            .entry(server.to_string())
            .or_insert_with(|| WindowTracker::new(self.limits.elicitation_per_server));

        if tracker.try_request() {
            RateLimitResult::Allowed {
                remaining: tracker.remaining(),
            }
        } else {
            RateLimitResult::Denied {
                retry_after: tracker
                    .retry_after()
                    .unwrap_or_else(|| Duration::seconds(1)),
            }
        }
    }

    /// Check and record a sampling request.
    pub async fn check_sampling(&self, server: &str) -> RateLimitResult {
        let mut trackers = self.sampling.write().await;
        let tracker = trackers
            .entry(server.to_string())
            .or_insert_with(|| WindowTracker::new(self.limits.sampling_per_server));

        if tracker.try_request() {
            RateLimitResult::Allowed {
                remaining: tracker.remaining(),
            }
        } else {
            RateLimitResult::Denied {
                retry_after: tracker
                    .retry_after()
                    .unwrap_or_else(|| Duration::seconds(1)),
            }
        }
    }

    /// Try to acquire a pending request slot.
    ///
    /// Returns a guard that releases the slot when dropped.
    pub async fn acquire_pending(&self) -> Option<PendingGuard> {
        let mut count = self.pending_count.write().await;
        if *count >= self.limits.global_pending_requests {
            return None;
        }
        *count += 1;
        Some(PendingGuard {
            counter: Arc::clone(&self.pending_count),
        })
    }

    /// Get current pending request count.
    pub async fn pending_count(&self) -> u32 {
        *self.pending_count.read().await
    }

    /// Get remaining elicitation requests for a server.
    pub async fn elicitation_remaining(&self, server: &str) -> u32 {
        let trackers = self.elicitation.read().await;
        trackers.get(server).map_or(
            self.limits.elicitation_per_server.max_requests,
            WindowTracker::remaining,
        )
    }

    /// Get remaining sampling requests for a server.
    pub async fn sampling_remaining(&self, server: &str) -> u32 {
        let trackers = self.sampling.read().await;
        trackers.get(server).map_or(
            self.limits.sampling_per_server.max_requests,
            WindowTracker::remaining,
        )
    }

    /// Reset all rate limits (for testing).
    pub async fn reset(&self) {
        self.elicitation.write().await.clear();
        self.sampling.write().await.clear();
        *self.pending_count.write().await = 0;
    }
}

/// Guard that releases a pending request slot when dropped.
#[derive(Debug)]
pub struct PendingGuard {
    counter: Arc<RwLock<u32>>,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        // We need to spawn a task to decrement since drop can't be async
        let counter = Arc::clone(&self.counter);
        tokio::spawn(async move {
            let mut count = counter.write().await;
            *count = count.saturating_sub(1);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_creation() {
        let limit = RateLimit::per_minute(60);
        assert_eq!(limit.max_requests, 60);
        assert_eq!(limit.window, Duration::minutes(1));

        let limit = RateLimit::per_second(10);
        assert_eq!(limit.max_requests, 10);
        assert_eq!(limit.window, Duration::seconds(1));
    }

    #[test]
    fn test_rate_limits_default() {
        let limits = RateLimits::default();
        assert_eq!(limits.elicitation_per_server.max_requests, 10);
        assert_eq!(limits.sampling_per_server.max_requests, 30);
        assert_eq!(limits.global_pending_requests, 100);
    }

    #[test]
    fn test_window_tracker() {
        let limit = RateLimit::new(3, Duration::seconds(1));
        let mut tracker = WindowTracker::new(limit);

        // First 3 requests should be allowed
        assert!(tracker.try_request());
        assert!(tracker.try_request());
        assert!(tracker.try_request());

        // 4th request should be denied
        assert!(!tracker.try_request());

        assert_eq!(tracker.remaining(), 0);
    }

    #[tokio::test]
    async fn test_rate_limiter_elicitation() {
        let limits = RateLimits {
            elicitation_per_server: RateLimit::new(2, Duration::seconds(1)),
            sampling_per_server: RateLimit::default(),
            global_pending_requests: 100,
        };
        let limiter = RateLimiter::new(limits);

        // First 2 requests allowed
        assert!(limiter.check_elicitation("server1").await.is_allowed());
        assert!(limiter.check_elicitation("server1").await.is_allowed());

        // 3rd request denied
        assert!(!limiter.check_elicitation("server1").await.is_allowed());

        // Different server has its own limit
        assert!(limiter.check_elicitation("server2").await.is_allowed());
    }

    #[tokio::test]
    async fn test_rate_limiter_pending() {
        let limits = RateLimits {
            elicitation_per_server: RateLimit::default(),
            sampling_per_server: RateLimit::default(),
            global_pending_requests: 2,
        };
        let limiter = RateLimiter::new(limits);

        // Acquire 2 slots
        let guard1 = limiter.acquire_pending().await;
        assert!(guard1.is_some());
        let guard2 = limiter.acquire_pending().await;
        assert!(guard2.is_some());

        // 3rd acquisition should fail
        let guard3 = limiter.acquire_pending().await;
        assert!(guard3.is_none());

        assert_eq!(limiter.pending_count().await, 2);

        // Drop a guard and try again
        drop(guard1);
        // Give the async drop task time to run
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let guard4 = limiter.acquire_pending().await;
        assert!(guard4.is_some());
    }

    #[tokio::test]
    async fn test_rate_limiter_reset() {
        let limiter = RateLimiter::new(RateLimits::default());

        limiter.check_elicitation("server").await;
        limiter.check_sampling("server").await;

        limiter.reset().await;

        assert_eq!(
            limiter.elicitation_remaining("server").await,
            10 // default max
        );
    }
}
