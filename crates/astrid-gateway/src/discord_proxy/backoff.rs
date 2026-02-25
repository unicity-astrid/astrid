//! Exponential backoff with full jitter for reconnection delays.
//!
//! Follows the AWS "Full Jitter" strategy:
//! `delay = random(0, min(cap, base * 2^attempt))`.

use std::time::Duration;

/// Exponential backoff calculator with full jitter.
pub(crate) struct Backoff {
    /// Base delay in milliseconds.
    base_ms: u64,
    /// Maximum delay cap in milliseconds.
    max_ms: u64,
    /// Current attempt number (0-indexed).
    attempt: u32,
}

impl Backoff {
    /// Create a new backoff calculator.
    pub(super) fn new(base_ms: u64, max_ms: u64) -> Self {
        Self {
            base_ms,
            max_ms,
            attempt: 0,
        }
    }

    /// Compute the next delay with full jitter and advance the attempt.
    pub(super) fn next_delay(&mut self) -> Duration {
        let exp = self
            .base_ms
            .saturating_mul(1u64.checked_shl(self.attempt).unwrap_or(u64::MAX));
        let capped = exp.min(self.max_ms);
        let jittered = if capped == 0 {
            0
        } else {
            fastrand::u64(0..=capped)
        };
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_millis(jittered)
    }

    /// Reset the attempt counter after a successful connection.
    pub(super) fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_delay_bounded_by_base() {
        let mut b = Backoff::new(1000, 60_000);
        for _ in 0..100 {
            b.attempt = 0;
            let delay = b.next_delay();
            assert!(delay <= Duration::from_millis(1000));
        }
    }

    #[test]
    fn delay_grows_exponentially() {
        let mut b = Backoff::new(1000, 60_000);
        // After attempt 0: max 1s
        let _ = b.next_delay();
        assert_eq!(b.attempt, 1);
        // After attempt 1: max 2s
        // After attempt 2: max 4s
        // etc.
    }

    #[test]
    fn delay_capped_at_max() {
        let mut b = Backoff::new(1000, 5000);
        // Advance many attempts.
        for _ in 0..20 {
            let delay = b.next_delay();
            assert!(delay <= Duration::from_millis(5000));
        }
    }

    #[test]
    fn reset_resets_attempt() {
        let mut b = Backoff::new(1000, 60_000);
        for _ in 0..5 {
            let _ = b.next_delay();
        }
        assert_eq!(b.attempt, 5);
        b.reset();
        assert_eq!(b.attempt, 0);
    }

    #[test]
    fn zero_base_produces_zero_delay() {
        let mut b = Backoff::new(0, 0);
        for _ in 0..10 {
            let delay = b.next_delay();
            assert_eq!(delay, Duration::ZERO);
        }
    }

    #[test]
    fn attempt_saturates() {
        let mut b = Backoff::new(1000, 60_000);
        b.attempt = u32::MAX;
        let delay = b.next_delay();
        assert!(delay <= Duration::from_millis(60_000));
        assert_eq!(b.attempt, u32::MAX);
    }
}
