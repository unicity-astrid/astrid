//! Verification - User Action Verification for Grants and Permissions
//!
//! This module handles the verification flow for permission grants.
//! When the LLM proposes a grant based on natural conversation, the system
//! must verify that the user actually intended to grant permission.
//!
//! # Key Types
//!
//! - [`VerificationRequest`] - Request for user to verify an action
//! - [`VerificationResponse`] - User's verification decision
//!
//! # Verification Flow
//!
//! ```text
//! User B: "you can tell them about the auth project"
//!                        │
//!                        ▼
//! LLM detects potential grant, proposes it with trigger_message_id
//!                        │
//!                        ▼
//! System fetches the message, confirms it exists and is from user_b
//!                        │
//!                        ▼
//! System sends VerificationRequest to user_b
//! "Share memories about 'auth project' with channel?"
//! [Confirm] [Deny]
//!                        │
//!                        ▼
//! User clicks [Confirm] → VerificationResponse::Confirmed
//! THIS creates the actual AccessGrant
//! ```

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use crate::input::MessageId;
use crate::types::RiskLevel;

/// Request for user to verify an action.
///
/// This is sent to a user when the system needs to confirm their intent,
/// such as granting memory access based on natural language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRequest {
    /// Unique request ID
    pub request_id: Uuid,
    /// What triggered this verification (the message the LLM interpreted)
    pub trigger_message_id: MessageId,
    /// Type of verification
    pub verification_type: VerificationType,
    /// Human-readable description
    pub description: String,
    /// Available options
    pub options: Vec<VerificationOption>,
    /// Risk level (determines delivery method)
    pub risk_level: RiskLevel,
    /// When this request expires
    pub expires_at: DateTime<Utc>,
    /// Metadata for the action being verified
    pub metadata: VerificationMetadata,
}

impl VerificationRequest {
    /// Create a new verification request.
    #[must_use]
    pub fn new(trigger_message_id: MessageId, verification_type: VerificationType) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            trigger_message_id,
            verification_type,
            description: String::new(),
            options: vec![
                VerificationOption::Confirm,
                VerificationOption::ConfirmTemporary {
                    duration: Duration::hours(24),
                },
                VerificationOption::Deny,
            ],
            risk_level: RiskLevel::Medium,
            // Safety: chrono Duration addition to DateTime cannot overflow for reasonable durations
            #[allow(clippy::arithmetic_side_effects)]
            expires_at: Utc::now() + Duration::minutes(5),
            metadata: VerificationMetadata::default(),
        }
    }

    /// Set the description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the risk level.
    #[must_use]
    pub fn with_risk_level(mut self, level: RiskLevel) -> Self {
        self.risk_level = level;
        self
    }

    /// Set custom options.
    #[must_use]
    pub fn with_options(mut self, options: Vec<VerificationOption>) -> Self {
        self.options = options;
        self
    }

    /// Set the expiration time.
    #[must_use]
    pub fn expires_in(mut self, duration: Duration) -> Self {
        // Safety: chrono Duration addition to DateTime cannot overflow for reasonable durations
        #[allow(clippy::arithmetic_side_effects)]
        {
            self.expires_at = Utc::now() + duration;
        }
        self
    }

    /// Set metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: VerificationMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Check if this request has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Check if this should be sent via DM based on risk level.
    #[must_use]
    pub fn should_use_dm(&self) -> bool {
        self.risk_level.requires_dm_verification()
    }
}

/// Type of verification being requested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationType {
    /// Granting access to memories
    MemoryGrant {
        /// Topic/scope of the grant
        topic_hint: String,
        /// Who would get access
        audience_description: String,
    },
    /// Revoking access to memories
    MemoryRevoke {
        /// What access is being revoked
        scope_description: String,
    },
    /// Deleting memories
    MemoryDelete {
        /// What would be deleted
        scope_description: String,
        /// Number of memories affected
        count: usize,
    },
    /// Identity linking
    IdentityLink {
        /// The frontend being linked
        frontend: String,
    },
    /// Generic action verification
    Generic {
        /// Action being verified
        action: String,
    },
}

impl fmt::Display for VerificationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemoryGrant { topic_hint, .. } => {
                write!(f, "memory_grant:{topic_hint}")
            },
            Self::MemoryRevoke { scope_description } => {
                write!(f, "memory_revoke:{scope_description}")
            },
            Self::MemoryDelete { count, .. } => {
                write!(f, "memory_delete:{count}")
            },
            Self::IdentityLink { frontend } => {
                write!(f, "identity_link:{frontend}")
            },
            Self::Generic { action } => {
                write!(f, "generic:{action}")
            },
        }
    }
}

/// Options available for verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOption {
    /// Confirm the action permanently (or with default expiry)
    Confirm,
    /// Confirm temporarily
    ConfirmTemporary {
        /// How long the grant should last
        #[serde(with = "duration_serde")]
        duration: Duration,
    },
    /// Deny the action
    Deny,
    /// Ask for more information
    NeedMoreInfo,
}

impl fmt::Display for VerificationOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confirm => write!(f, "Confirm"),
            Self::ConfirmTemporary { duration } => {
                let hours = duration.num_hours();
                if hours >= 24 {
                    write!(f, "Confirm for {} days", hours / 24)
                } else {
                    write!(f, "Confirm for {hours} hours")
                }
            },
            Self::Deny => write!(f, "Deny"),
            Self::NeedMoreInfo => write!(f, "Need more info"),
        }
    }
}

mod duration_serde {
    use chrono::Duration;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.num_seconds().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let seconds = i64::deserialize(deserializer)?;
        Ok(Duration::seconds(seconds))
    }
}

/// Metadata about the action being verified.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationMetadata {
    /// Topic/subject of the verification
    pub topic: Option<String>,
    /// Who initiated the action
    pub initiator_id: Option<Uuid>,
    /// Who would be affected
    pub affected_user_ids: Vec<Uuid>,
    /// Context where this was triggered
    pub context_id: Option<String>,
    /// Number of items affected
    pub affected_count: Option<usize>,
    /// Additional key-value metadata
    pub extra: std::collections::HashMap<String, String>,
}

impl VerificationMetadata {
    /// Create new metadata with a topic.
    #[must_use]
    pub fn with_topic(topic: impl Into<String>) -> Self {
        Self {
            topic: Some(topic.into()),
            ..Default::default()
        }
    }

    /// Set the initiator.
    #[must_use]
    pub fn initiator(mut self, id: Uuid) -> Self {
        self.initiator_id = Some(id);
        self
    }

    /// Add an affected user.
    #[must_use]
    pub fn affects(mut self, user_id: Uuid) -> Self {
        self.affected_user_ids.push(user_id);
        self
    }

    /// Set the context.
    #[must_use]
    pub fn in_context(mut self, context_id: impl Into<String>) -> Self {
        self.context_id = Some(context_id.into());
        self
    }

    /// Set the affected count.
    #[must_use]
    pub fn with_count(mut self, count: usize) -> Self {
        self.affected_count = Some(count);
        self
    }

    /// Add extra metadata.
    #[must_use]
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }
}

/// Response to a verification request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResponse {
    /// Request ID this responds to
    pub request_id: Uuid,
    /// The option selected
    pub decision: VerificationDecision,
    /// When the decision was made
    pub decided_at: DateTime<Utc>,
    /// Frontend-specific action ID (button click, reaction, etc.)
    pub action_id: Option<String>,
    /// Optional message from the user
    pub user_message: Option<String>,
}

impl VerificationResponse {
    /// Create a confirmed response.
    #[must_use]
    pub fn confirmed(request_id: Uuid) -> Self {
        Self {
            request_id,
            decision: VerificationDecision::Confirmed { expiry: None },
            decided_at: Utc::now(),
            action_id: None,
            user_message: None,
        }
    }

    /// Create a confirmed response with expiry.
    #[must_use]
    pub fn confirmed_temporary(request_id: Uuid, duration: Duration) -> Self {
        Self {
            request_id,
            decision: VerificationDecision::Confirmed {
                // Safety: chrono Duration addition to DateTime cannot overflow for reasonable durations
                #[allow(clippy::arithmetic_side_effects)]
                expiry: Some(Utc::now() + duration),
            },
            decided_at: Utc::now(),
            action_id: None,
            user_message: None,
        }
    }

    /// Create a denied response.
    #[must_use]
    pub fn denied(request_id: Uuid) -> Self {
        Self {
            request_id,
            decision: VerificationDecision::Denied,
            decided_at: Utc::now(),
            action_id: None,
            user_message: None,
        }
    }

    /// Create an expired response.
    #[must_use]
    pub fn expired(request_id: Uuid) -> Self {
        Self {
            request_id,
            decision: VerificationDecision::Expired,
            decided_at: Utc::now(),
            action_id: None,
            user_message: None,
        }
    }

    /// Set the action ID.
    #[must_use]
    pub fn with_action_id(mut self, action_id: impl Into<String>) -> Self {
        self.action_id = Some(action_id.into());
        self
    }

    /// Set a user message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.user_message = Some(message.into());
        self
    }

    /// Check if this is a confirmation.
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        matches!(self.decision, VerificationDecision::Confirmed { .. })
    }

    /// Check if this is a denial.
    #[must_use]
    pub fn is_denied(&self) -> bool {
        matches!(self.decision, VerificationDecision::Denied)
    }

    /// Get the expiry time if this is a temporary confirmation.
    #[must_use]
    pub fn expiry(&self) -> Option<DateTime<Utc>> {
        match &self.decision {
            VerificationDecision::Confirmed { expiry } => *expiry,
            _ => None,
        }
    }
}

/// The decision made in response to verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationDecision {
    /// User confirmed the action
    Confirmed {
        /// Optional expiry for the confirmation
        expiry: Option<DateTime<Utc>>,
    },
    /// User denied the action
    Denied,
    /// Request expired before user responded
    Expired,
    /// User requested more information
    NeedMoreInfo,
}

impl fmt::Display for VerificationDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confirmed { expiry: None } => write!(f, "confirmed"),
            Self::Confirmed { expiry: Some(exp) } => write!(f, "confirmed_until:{exp}"),
            Self::Denied => write!(f, "denied"),
            Self::Expired => write!(f, "expired"),
            Self::NeedMoreInfo => write!(f, "need_more_info"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_request_creation() {
        let msg_id = MessageId::discord(123_456_789);
        let req = VerificationRequest::new(
            msg_id.clone(),
            VerificationType::MemoryGrant {
                topic_hint: "auth project".to_string(),
                audience_description: "channel participants".to_string(),
            },
        )
        .with_description("Share memories about 'auth project' with this channel?")
        .with_risk_level(RiskLevel::Medium);

        assert_eq!(req.trigger_message_id, msg_id);
        assert!(!req.is_expired());
        assert!(!req.should_use_dm()); // Medium risk doesn't require DM
    }

    #[test]
    fn test_verification_request_expiry() {
        let msg_id = MessageId::discord(123);
        let req = VerificationRequest::new(
            msg_id,
            VerificationType::Generic {
                action: "test".to_string(),
            },
        )
        .expires_in(Duration::seconds(-1)); // Already expired

        assert!(req.is_expired());
    }

    #[test]
    fn test_verification_response_confirmed() {
        let request_id = Uuid::new_v4();
        let resp = VerificationResponse::confirmed(request_id).with_action_id("btn_click_123");

        assert!(resp.is_confirmed());
        assert!(!resp.is_denied());
        assert!(resp.expiry().is_none());
        assert_eq!(resp.action_id, Some("btn_click_123".to_string()));
    }

    #[test]
    fn test_verification_response_temporary() {
        let request_id = Uuid::new_v4();
        let resp = VerificationResponse::confirmed_temporary(request_id, Duration::hours(24));

        assert!(resp.is_confirmed());
        assert!(resp.expiry().is_some());
    }

    #[test]
    fn test_verification_response_denied() {
        let request_id = Uuid::new_v4();
        let resp = VerificationResponse::denied(request_id);

        assert!(!resp.is_confirmed());
        assert!(resp.is_denied());
    }

    #[test]
    fn test_verification_option_display() {
        assert_eq!(VerificationOption::Confirm.to_string(), "Confirm");
        assert_eq!(VerificationOption::Deny.to_string(), "Deny");
        assert_eq!(
            VerificationOption::ConfirmTemporary {
                duration: Duration::hours(24)
            }
            .to_string(),
            "Confirm for 1 days"
        );
        assert_eq!(
            VerificationOption::ConfirmTemporary {
                duration: Duration::hours(6)
            }
            .to_string(),
            "Confirm for 6 hours"
        );
    }

    #[test]
    fn test_verification_metadata() {
        let user_id = Uuid::new_v4();
        let metadata = VerificationMetadata::with_topic("project discussion")
            .initiator(user_id)
            .affects(Uuid::new_v4())
            .with_count(5)
            .with_extra("source", "conversation");

        assert_eq!(metadata.topic, Some("project discussion".to_string()));
        assert_eq!(metadata.initiator_id, Some(user_id));
        assert_eq!(metadata.affected_user_ids.len(), 1);
        assert_eq!(metadata.affected_count, Some(5));
        assert!(metadata.extra.contains_key("source"));
    }

    #[test]
    fn test_verification_type_display() {
        let grant = VerificationType::MemoryGrant {
            topic_hint: "auth".to_string(),
            audience_description: "team".to_string(),
        };
        assert!(grant.to_string().contains("memory_grant"));

        let delete = VerificationType::MemoryDelete {
            scope_description: "old stuff".to_string(),
            count: 10,
        };
        assert!(delete.to_string().contains("memory_delete:10"));
    }
}
