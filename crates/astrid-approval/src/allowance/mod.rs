//! Allowance types and store for pre-approved action patterns.
//!
//! An [`Allowance`] grants pre-approved access for actions matching a specific
//! pattern. Created when users select "Allow Session" or "Create Allowance"
//! during approval flows.
//!
//! The [`AllowanceStore`] holds active allowances in memory, supporting
//! pattern-based matching, use tracking, expiration cleanup, and session clearing.

mod pattern;
mod store;

pub use pattern::AllowancePattern;
pub use store::AllowanceStore;

use astrid_core::types::Timestamp;
use astrid_crypto::Signature;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for an allowance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AllowanceId(pub Uuid);

impl AllowanceId {
    /// Create a new random allowance ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AllowanceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AllowanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "allowance:{}", self.0)
    }
}

/// An allowance granting pre-approved access for actions matching a pattern.
///
/// Allowances are created during approval flows:
/// - **Session allowances** (`session_only: true`): Cleared when the session ends.
/// - **Persistent allowances**: Survive across sessions (backed by capability tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Allowance {
    /// Unique allowance identifier.
    pub id: AllowanceId,
    /// Pattern describing what actions this allowance covers.
    pub action_pattern: AllowancePattern,
    /// When the allowance was created.
    pub created_at: Timestamp,
    /// When the allowance expires (None = no expiration within scope).
    pub expires_at: Option<Timestamp>,
    /// Maximum number of uses (None = unlimited).
    pub max_uses: Option<u32>,
    /// Remaining uses (None = unlimited, decremented on each use).
    pub uses_remaining: Option<u32>,
    /// Whether this allowance is scoped to the current session only.
    pub session_only: bool,
    /// Workspace root this allowance is scoped to (None = not workspace-scoped).
    pub workspace_root: Option<PathBuf>,
    /// Cryptographic signature proving this allowance was legitimately created.
    pub signature: Signature,
}

impl Allowance {
    /// Check if the allowance has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.expires_at.as_ref().is_some_and(Timestamp::is_past)
    }

    /// Check if the allowance has uses remaining.
    #[must_use]
    pub fn has_uses_remaining(&self) -> bool {
        self.uses_remaining.is_none_or(|r| r > 0)
    }

    /// Check if the allowance is still valid (not expired, has uses).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.is_expired() && self.has_uses_remaining()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrid_crypto::KeyPair;

    #[test]
    fn test_allowance_id() {
        let id1 = AllowanceId::new();
        let id2 = AllowanceId::new();
        assert_ne!(id1, id2);
        assert!(id1.to_string().starts_with("allowance:"));
    }

    #[test]
    fn test_allowance_valid_no_limits() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test-allowance"),
        };
        assert!(!allowance.is_expired());
        assert!(allowance.has_uses_remaining());
        assert!(allowance.is_valid());
    }

    #[test]
    fn test_allowance_expired() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            created_at: Timestamp::from_datetime(chrono::Utc::now() - chrono::Duration::hours(2)),
            expires_at: Some(Timestamp::from_datetime(
                chrono::Utc::now() - chrono::Duration::hours(1),
            )),
            max_uses: None,
            uses_remaining: None,
            session_only: false,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        assert!(allowance.is_expired());
        assert!(!allowance.is_valid());
    }

    #[test]
    fn test_allowance_uses_exhausted() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: Some(5),
            uses_remaining: Some(0),
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        assert!(!allowance.has_uses_remaining());
        assert!(!allowance.is_valid());
    }

    #[test]
    fn test_allowance_uses_remaining() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ServerTools {
                server: "test".to_string(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: Some(5),
            uses_remaining: Some(3),
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        assert!(allowance.has_uses_remaining());
        assert!(allowance.is_valid());
    }

    #[test]
    fn test_allowance_serialization_roundtrip() {
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::ExactTool {
                server: "test".to_string(),
                tool: "test_tool".to_string(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test-allowance"),
        };
        let json = serde_json::to_string(&allowance).unwrap();
        let deserialized: Allowance = serde_json::from_str(&json).unwrap();
        assert_eq!(allowance.id, deserialized.id);
        assert_eq!(allowance.session_only, deserialized.session_only);
    }
}
