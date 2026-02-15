//! Health checks for the gateway.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Overall health state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthState {
    /// All systems healthy.
    Healthy,
    /// Some non-critical issues.
    Degraded,
    /// Critical issues.
    Unhealthy,
    /// Unknown state.
    Unknown,
}

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Unhealthy => write!(f, "unhealthy"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Result of a single health check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Name of the component being checked.
    pub component: String,

    /// Health state.
    pub state: HealthState,

    /// Human-readable message.
    pub message: Option<String>,

    /// Check duration.
    pub duration_ms: u64,

    /// When this check was performed.
    pub checked_at: DateTime<Utc>,

    /// Additional details.
    #[serde(default)]
    pub details: HashMap<String, serde_json::Value>,
}

impl HealthCheck {
    /// Create a healthy check result.
    #[must_use]
    pub fn healthy(component: impl Into<String>, duration: Duration) -> Self {
        Self {
            component: component.into(),
            state: HealthState::Healthy,
            message: None,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            checked_at: Utc::now(),
            details: HashMap::new(),
        }
    }

    /// Create an unhealthy check result.
    #[must_use]
    pub fn unhealthy(
        component: impl Into<String>,
        message: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            component: component.into(),
            state: HealthState::Unhealthy,
            message: Some(message.into()),
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            checked_at: Utc::now(),
            details: HashMap::new(),
        }
    }

    /// Create a degraded check result.
    #[must_use]
    pub fn degraded(
        component: impl Into<String>,
        message: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            component: component.into(),
            state: HealthState::Degraded,
            message: Some(message.into()),
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            checked_at: Utc::now(),
            details: HashMap::new(),
        }
    }

    /// Add a detail.
    #[must_use]
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(v) = serde_json::to_value(value) {
            self.details.insert(key.into(), v);
        }
        self
    }

    /// Add a message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// Overall health status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Overall state.
    pub state: HealthState,

    /// When this status was computed.
    pub checked_at: DateTime<Utc>,

    /// Individual check results.
    pub checks: Vec<HealthCheck>,

    /// Gateway uptime.
    pub uptime_secs: u64,

    /// Version information.
    pub version: String,
}

impl HealthStatus {
    /// Create a new health status from check results.
    #[must_use]
    pub fn from_checks(
        checks: Vec<HealthCheck>,
        uptime: Duration,
        version: impl Into<String>,
    ) -> Self {
        let state = Self::aggregate_state(&checks);

        Self {
            state,
            checked_at: Utc::now(),
            checks,
            uptime_secs: uptime.as_secs(),
            version: version.into(),
        }
    }

    /// Aggregate individual check states into overall state.
    fn aggregate_state(checks: &[HealthCheck]) -> HealthState {
        if checks.is_empty() {
            return HealthState::Unknown;
        }

        let has_unhealthy = checks.iter().any(|c| c.state == HealthState::Unhealthy);
        let has_degraded = checks.iter().any(|c| c.state == HealthState::Degraded);
        let has_unknown = checks.iter().any(|c| c.state == HealthState::Unknown);

        if has_unhealthy {
            HealthState::Unhealthy
        } else if has_degraded || has_unknown {
            HealthState::Degraded
        } else {
            HealthState::Healthy
        }
    }

    /// Check if healthy.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.state == HealthState::Healthy
    }

    /// Get checks by state.
    #[must_use]
    pub fn checks_by_state(&self, state: HealthState) -> Vec<&HealthCheck> {
        self.checks.iter().filter(|c| c.state == state).collect()
    }

    /// Get unhealthy checks.
    #[must_use]
    pub fn unhealthy_checks(&self) -> Vec<&HealthCheck> {
        self.checks_by_state(HealthState::Unhealthy)
    }
}

/// Run health checks for all gateway components.
pub async fn run_health_checks(
    agent_count: usize,
    mcp_server_count: usize,
    pending_approvals: usize,
    audit_healthy: bool,
    uptime: Duration,
    version: &str,
) -> HealthStatus {
    let mut checks = Vec::new();

    // Agent manager check
    let start = std::time::Instant::now();
    let agent_check = if agent_count > 0 {
        HealthCheck::healthy("agent_manager", start.elapsed())
            .with_detail("agent_count", agent_count)
    } else {
        HealthCheck::degraded("agent_manager", "no agents running", start.elapsed())
    };
    checks.push(agent_check);

    // MCP check
    let start = std::time::Instant::now();
    let mcp_check = if mcp_server_count > 0 {
        HealthCheck::healthy("mcp", start.elapsed()).with_detail("server_count", mcp_server_count)
    } else {
        HealthCheck::degraded("mcp", "no MCP servers available", start.elapsed())
    };
    checks.push(mcp_check);

    // Approval queue check
    let start = std::time::Instant::now();
    let approval_check = if pending_approvals < 100 {
        HealthCheck::healthy("approval_queue", start.elapsed())
            .with_detail("pending", pending_approvals)
    } else {
        HealthCheck::degraded(
            "approval_queue",
            format!("{pending_approvals} pending approvals"),
            start.elapsed(),
        )
    };
    checks.push(approval_check);

    // Audit check
    let start = std::time::Instant::now();
    let audit_check = if audit_healthy {
        HealthCheck::healthy("audit", start.elapsed())
    } else {
        HealthCheck::unhealthy("audit", "audit log unavailable", start.elapsed())
    };
    checks.push(audit_check);

    HealthStatus::from_checks(checks, uptime, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_state_display() {
        assert_eq!(HealthState::Healthy.to_string(), "healthy");
        assert_eq!(HealthState::Degraded.to_string(), "degraded");
        assert_eq!(HealthState::Unhealthy.to_string(), "unhealthy");
    }

    #[test]
    fn test_health_check_creation() {
        let check = HealthCheck::healthy("test", Duration::from_millis(10));
        assert_eq!(check.component, "test");
        assert_eq!(check.state, HealthState::Healthy);
        assert!(check.message.is_none());

        let check = HealthCheck::unhealthy("test", "error", Duration::from_millis(10));
        assert_eq!(check.state, HealthState::Unhealthy);
        assert_eq!(check.message, Some("error".into()));
    }

    #[test]
    fn test_health_check_details() {
        let check = HealthCheck::healthy("test", Duration::from_millis(10))
            .with_detail("count", 42)
            .with_message("all good");

        assert_eq!(check.details.get("count"), Some(&serde_json::json!(42)));
        assert_eq!(check.message, Some("all good".into()));
    }

    #[test]
    fn test_health_status_aggregation() {
        let checks = vec![
            HealthCheck::healthy("a", Duration::ZERO),
            HealthCheck::healthy("b", Duration::ZERO),
        ];
        let status = HealthStatus::from_checks(checks, Duration::from_secs(60), "1.0.0");
        assert_eq!(status.state, HealthState::Healthy);

        let checks = vec![
            HealthCheck::healthy("a", Duration::ZERO),
            HealthCheck::degraded("b", "issue", Duration::ZERO),
        ];
        let status = HealthStatus::from_checks(checks, Duration::from_secs(60), "1.0.0");
        assert_eq!(status.state, HealthState::Degraded);

        let checks = vec![
            HealthCheck::healthy("a", Duration::ZERO),
            HealthCheck::unhealthy("b", "error", Duration::ZERO),
        ];
        let status = HealthStatus::from_checks(checks, Duration::from_secs(60), "1.0.0");
        assert_eq!(status.state, HealthState::Unhealthy);
    }

    #[test]
    fn test_health_status_helpers() {
        let checks = vec![
            HealthCheck::healthy("a", Duration::ZERO),
            HealthCheck::unhealthy("b", "error", Duration::ZERO),
        ];
        let status = HealthStatus::from_checks(checks, Duration::from_secs(60), "1.0.0");

        assert!(!status.is_healthy());
        assert_eq!(status.unhealthy_checks().len(), 1);
        assert_eq!(status.unhealthy_checks()[0].component, "b");
    }

    #[tokio::test]
    async fn test_run_health_checks() {
        let status = run_health_checks(
            1,    // agent_count
            2,    // mcp_server_count
            0,    // pending_approvals
            true, // audit_healthy
            Duration::from_secs(3600),
            "0.1.0",
        )
        .await;

        assert_eq!(status.state, HealthState::Healthy);
        assert_eq!(status.checks.len(), 4);
        assert_eq!(status.version, "0.1.0");
    }
}
