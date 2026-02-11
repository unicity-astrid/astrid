//! Common types used throughout Astralis.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Unique identifier for an agent instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Create a new random agent ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create an agent ID from a UUID.
    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "agent:{}", self.0)
    }
}

/// Unique identifier for a session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Create a new random session ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a session ID from a UUID.
    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session:{}", self.0)
    }
}

/// Unique identifier for a capability token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenId(pub Uuid);

impl TokenId {
    /// Create a new random token ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a token ID from a UUID.
    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for TokenId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TokenId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "token:{}", self.0)
    }
}

/// Timestamp wrapper for consistent handling throughout Astralis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub DateTime<Utc>);

impl Timestamp {
    /// Get the current timestamp.
    #[must_use]
    pub fn now() -> Self {
        Self(Utc::now())
    }

    /// Create a timestamp from a `DateTime<Utc>`.
    #[must_use]
    pub fn from_datetime(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    /// Check if this timestamp is in the past.
    #[must_use]
    pub fn is_past(&self) -> bool {
        self.0 < Utc::now()
    }

    /// Check if this timestamp is in the future.
    #[must_use]
    pub fn is_future(&self) -> bool {
        self.0 > Utc::now()
    }

    /// Get the inner `DateTime<Utc>`.
    #[must_use]
    pub fn into_inner(self) -> DateTime<Utc> {
        self.0
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::now()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.format("%Y-%m-%dT%H:%M:%SZ"))
    }
}

impl From<DateTime<Utc>> for Timestamp {
    fn from(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }
}

impl From<Timestamp> for DateTime<Utc> {
    fn from(ts: Timestamp) -> Self {
        ts.0
    }
}

/// Permission types for resource access control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read access to a resource
    Read,
    /// Write access to a resource
    Write,
    /// Execute access (for tools/commands)
    Execute,
    /// Delete access to a resource
    Delete,
    /// Invoke access (for MCP tools)
    Invoke,
    /// List access (for directories/collections)
    List,
    /// Create access (for new resources)
    Create,
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => write!(f, "read"),
            Self::Write => write!(f, "write"),
            Self::Execute => write!(f, "execute"),
            Self::Delete => write!(f, "delete"),
            Self::Invoke => write!(f, "invoke"),
            Self::List => write!(f, "list"),
            Self::Create => write!(f, "create"),
        }
    }
}

/// Risk level classification for operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Low risk - typically allowed without explicit approval
    Low,
    /// Medium risk - may require approval depending on context
    Medium,
    /// High risk - requires explicit approval
    High,
    /// Critical risk - requires elevated approval with additional verification
    Critical,
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

impl RiskLevel {
    /// Check if this risk level requires user approval by default.
    #[must_use]
    pub fn requires_approval(&self) -> bool {
        matches!(self, Self::High | Self::Critical)
    }

    /// Check if this risk level requires DM verification (vs inline).
    #[must_use]
    pub fn requires_dm_verification(&self) -> bool {
        matches!(self, Self::Critical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id() {
        let id1 = AgentId::new();
        let id2 = AgentId::new();
        assert_ne!(id1, id2);

        let display = id1.to_string();
        assert!(display.starts_with("agent:"));
    }

    #[test]
    fn test_session_id() {
        let id = SessionId::new();
        let display = id.to_string();
        assert!(display.starts_with("session:"));
    }

    #[test]
    fn test_token_id() {
        let id = TokenId::new();
        let display = id.to_string();
        assert!(display.starts_with("token:"));
    }

    #[test]
    fn test_timestamp() {
        let ts = Timestamp::now();
        assert!(!ts.is_future());

        let past = Timestamp::from_datetime(Utc::now() - chrono::Duration::hours(1));
        assert!(past.is_past());
    }

    #[test]
    fn test_permission_display() {
        assert_eq!(Permission::Read.to_string(), "read");
        assert_eq!(Permission::Write.to_string(), "write");
        assert_eq!(Permission::Execute.to_string(), "execute");
    }

    #[test]
    fn test_risk_level_ordering() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
        assert!(RiskLevel::High < RiskLevel::Critical);
    }

    #[test]
    fn test_risk_level_requires_approval() {
        assert!(!RiskLevel::Low.requires_approval());
        assert!(!RiskLevel::Medium.requires_approval());
        assert!(RiskLevel::High.requires_approval());
        assert!(RiskLevel::Critical.requires_approval());
    }
}
