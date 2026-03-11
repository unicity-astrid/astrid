//! Retry utilities with exponential backoff.
//!
//! This module provides configurable retry logic for transient failures,
//! commonly used for network operations and external service calls.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for retry behavior with exponential backoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries, just the initial attempt).
    pub max_attempts: u32,
    /// Initial delay before the first retry.
    pub initial_delay: Duration,
    /// Maximum delay between retries (caps the exponential growth).
    pub max_delay: Duration,
    /// Base for exponential backoff (typically 2.0).
    pub exponential_base: f64,
    /// Optional jitter factor (0.0 to 1.0) to randomize delays.
    #[serde(default)]
    pub jitter_factor: f64,
}

impl RetryConfig {
    /// Creates a new retry configuration.
    #[must_use]
    pub fn new(
        max_attempts: u32,
        initial_delay: Duration,
        max_delay: Duration,
        exponential_base: f64,
    ) -> Self {
        Self {
            max_attempts,
            initial_delay,
            max_delay,
            exponential_base,
            jitter_factor: 0.0,
        }
    }

    /// Creates a configuration with no retries.
    #[must_use]
    pub const fn no_retry() -> Self {
        Self {
            max_attempts: 0,
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            exponential_base: 2.0,
            jitter_factor: 0.0,
        }
    }

    /// Creates a configuration suitable for quick local operations.
    #[must_use]
    pub fn fast() -> Self {
        Self::new(
            3,
            Duration::from_millis(10),
            Duration::from_millis(100),
            2.0,
        )
    }

    /// Creates a configuration suitable for network operations.
    #[must_use]
    pub fn network() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            exponential_base: 2.0,
            jitter_factor: 0.1,
        }
    }

    /// Creates a configuration suitable for external API calls.
    #[must_use]
    pub fn api() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            exponential_base: 2.0,
            jitter_factor: 0.2,
        }
    }

    /// Sets the jitter factor and returns self for builder-style configuration.
    #[must_use]
    pub const fn with_jitter(mut self, factor: f64) -> Self {
        self.jitter_factor = factor;
        self
    }

    /// Calculates the delay for a given attempt number (0-indexed).
    ///
    /// Returns `Duration::ZERO` for attempt 0, then exponentially increasing
    /// delays for subsequent attempts, capped at `max_delay`.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        // Calculate base delay with exponential backoff.
        // Precision loss is acceptable for delay calculations.
        let exponent = i32::try_from(attempt.saturating_sub(1)).unwrap_or(i32::MAX);
        let base_delay_ms =
            self.initial_delay.as_millis() as f64 * self.exponential_base.powi(exponent);

        let capped_delay_ms = base_delay_ms.min(self.max_delay.as_millis() as f64);

        // Safe: delays are always positive and within reasonable bounds
        Duration::from_millis(capped_delay_ms.max(0.0) as u64)
    }

    /// Calculates the delay for a given attempt with jitter applied.
    ///
    /// Jitter helps prevent thundering herd problems when many clients
    /// retry simultaneously.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn delay_for_attempt_with_jitter(&self, attempt: u32, random_factor: f64) -> Duration {
        let base_delay = self.delay_for_attempt(attempt);

        if self.jitter_factor <= 0.0 {
            return base_delay;
        }

        // random_factor should be between 0.0 and 1.0
        let random_factor = random_factor.clamp(0.0, 1.0);

        // Apply jitter: delay * (1 - jitter_factor + 2 * jitter_factor * random)
        // This gives a range of [delay * (1 - jitter), delay * (1 + jitter)]
        let jitter_multiplier =
            1.0 - self.jitter_factor + (2.0 * self.jitter_factor * random_factor);

        let jittered_ms = base_delay.as_millis() as f64 * jitter_multiplier;

        // Safe: jittered delays are always positive
        Duration::from_millis(jittered_ms.max(0.0) as u64)
    }

    /// Returns true if more attempts are allowed given the current attempt count.
    #[must_use]
    pub fn should_retry(&self, current_attempt: u32) -> bool {
        current_attempt < self.max_attempts
    }

    /// Returns an iterator over the delays for all retry attempts.
    pub fn delays(&self) -> impl Iterator<Item = Duration> + '_ {
        (1..=self.max_attempts).map(|attempt| self.delay_for_attempt(attempt))
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self::network()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_calculation() {
        let config = RetryConfig::new(5, Duration::from_millis(100), Duration::from_secs(10), 2.0);

        assert_eq!(config.delay_for_attempt(0), Duration::ZERO);
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(100));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(200));
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(400));
        assert_eq!(config.delay_for_attempt(4), Duration::from_millis(800));
    }

    #[test]
    fn delay_caps_at_max() {
        let config = RetryConfig::new(
            10,
            Duration::from_millis(100),
            Duration::from_millis(500),
            2.0,
        );

        // Should cap at 500ms
        assert_eq!(config.delay_for_attempt(5), Duration::from_millis(500));
        assert_eq!(config.delay_for_attempt(10), Duration::from_millis(500));
    }

    #[test]
    fn should_retry_logic() {
        let config = RetryConfig::new(3, Duration::from_millis(100), Duration::from_secs(1), 2.0);

        assert!(config.should_retry(0));
        assert!(config.should_retry(1));
        assert!(config.should_retry(2));
        assert!(!config.should_retry(3));
        assert!(!config.should_retry(4));
    }

    #[test]
    fn no_retry_config() {
        let config = RetryConfig::no_retry();

        assert!(!config.should_retry(0));
        assert_eq!(config.delays().count(), 0);
    }

    #[test]
    fn jitter_application() {
        let config = RetryConfig::network();

        let base_delay = config.delay_for_attempt(1);
        let jittered_low = config.delay_for_attempt_with_jitter(1, 0.0);
        let jittered_high = config.delay_for_attempt_with_jitter(1, 1.0);

        // With jitter_factor of 0.1:
        // low = base * 0.9, high = base * 1.1
        assert!(jittered_low < base_delay);
        assert!(jittered_high > base_delay);
    }
}
