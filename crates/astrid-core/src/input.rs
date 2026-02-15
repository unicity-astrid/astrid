//! Input Attribution - Tagged Messages and Context Identifiers
//!
//! Every piece of input the LLM receives is tagged with source attribution.
//! This is foundational to Astrid security - the system always knows WHO
//! said WHAT and in WHAT context.
//!
//! # Key Types
//!
//! - [`TaggedMessage`] - A message with full attribution
//! - [`MessageId`] - Unique identifier for a message
//! - [`ContextIdentifier`] - Where the message came from
//!
//! # Example
//!
//! ```rust
//! use astrid_core::input::{TaggedMessage, MessageId, ContextIdentifier};
//! use astrid_core::identity::{AstridUserId, FrontendType};
//!
//! let msg_id = MessageId::new("discord", "123456789");
//! let user = AstridUserId::new();
//! let context = ContextIdentifier::DirectMessage {
//!     participant_ids: vec![user.id],
//! };
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use crate::identity::{AstridUserId, FrontendType};
use crate::utils::truncate_to_boundary;

/// Unique message identifier.
///
/// Combines the frontend type with the frontend-specific message ID
/// to create a globally unique identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId {
    /// Frontend type (discord, telegram, cli, etc.)
    pub frontend: String,
    /// Frontend-specific message ID
    pub id: String,
}

impl MessageId {
    /// Create a new message ID.
    #[must_use]
    pub fn new(frontend: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            frontend: frontend.into(),
            id: id.into(),
        }
    }

    /// Create a message ID for a CLI input.
    #[must_use]
    pub fn cli(sequence: u64) -> Self {
        Self {
            frontend: "cli".to_string(),
            id: format!("seq_{sequence}"),
        }
    }

    /// Create a message ID for a Discord message.
    #[must_use]
    pub fn discord(snowflake: u64) -> Self {
        Self {
            frontend: "discord".to_string(),
            id: snowflake.to_string(),
        }
    }

    /// Create a message ID from a frontend type.
    #[must_use]
    pub fn from_frontend(frontend: &FrontendType, id: impl Into<String>) -> Self {
        Self {
            frontend: frontend.to_string(),
            id: id.into(),
        }
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.frontend, self.id)
    }
}

/// Every piece of input has verifiable source attribution.
///
/// This is the core unit of input to the Astrid system. The LLM sees
/// messages with full attribution, and the system uses this to verify
/// any claims made about who said what.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaggedMessage {
    /// Unique ID for this message (from frontend - e.g., Discord message ID)
    pub message_id: MessageId,

    /// Who sent this message (Astrid identity - spans frontends)
    pub astrid_user_id: AstridUserId,

    /// Frontend-specific user ID (for display/audit)
    pub frontend_user_id: String,

    /// Which frontend this came from
    pub frontend: FrontendType,

    /// Where this message came from (channel context)
    pub context: ContextIdentifier,

    /// When the message was sent
    pub timestamp: DateTime<Utc>,

    /// The actual content
    pub content: String,

    /// Optional signature (if user has registered ed25519 key)
    /// Stored as base64-encoded 64-byte signature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_signature: Option<String>,
}

impl TaggedMessage {
    /// Create a new tagged message.
    #[must_use]
    #[allow(clippy::similar_names)] // context and content are both meaningful names
    pub fn new(
        message_id: MessageId,
        astrid_user_id: AstridUserId,
        frontend_user_id: impl Into<String>,
        frontend: FrontendType,
        context: ContextIdentifier,
        content: impl Into<String>,
    ) -> Self {
        Self {
            message_id,
            astrid_user_id,
            frontend_user_id: frontend_user_id.into(),
            frontend,
            context,
            timestamp: Utc::now(),
            content: content.into(),
            user_signature: None,
        }
    }

    /// Add a user signature to this message.
    #[must_use]
    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.user_signature = Some(signature.into());
        self
    }

    /// Check if this message is from a DM context.
    #[must_use]
    pub fn is_dm(&self) -> bool {
        matches!(self.context, ContextIdentifier::DirectMessage { .. })
    }

    /// Check if this message has a valid signature.
    #[must_use]
    pub fn is_signed(&self) -> bool {
        self.user_signature.is_some() && self.astrid_user_id.has_signing_key()
    }

    /// Get the data that should be signed for this message.
    #[must_use]
    pub fn signable_data(&self) -> Vec<u8> {
        // Sign: message_id + user_id + timestamp + content
        let mut data = Vec::new();
        data.extend(self.message_id.to_string().as_bytes());
        data.extend(self.astrid_user_id.id.as_bytes());
        data.extend(self.timestamp.to_rfc3339().as_bytes());
        data.extend(self.content.as_bytes());
        data
    }
}

impl fmt::Display for TaggedMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} ({}): {}",
            self.message_id,
            self.astrid_user_id,
            self.context,
            if self.content.len() > 50 {
                format!("{}...", truncate_to_boundary(&self.content, 50))
            } else {
                self.content.clone()
            }
        )
    }
}

/// Context identifier - where a message came from.
///
/// This determines the default access rules for memories created
/// from this context.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextIdentifier {
    /// Direct message between user and agent
    DirectMessage {
        /// Participant user IDs (excluding agent)
        participant_ids: Vec<Uuid>,
    },

    /// Guild/server channel (Discord, Slack, etc.)
    GuildChannel {
        /// Guild/server ID
        guild_id: String,
        /// Channel ID
        channel_id: String,
    },

    /// Private group chat
    GroupChat {
        /// Group ID
        group_id: String,
        /// Participant user IDs
        participant_ids: Vec<Uuid>,
    },

    /// Web session
    WebSession {
        /// Session ID
        session_id: String,
        /// User ID
        user_id: Uuid,
    },

    /// CLI session
    CliSession {
        /// Session ID
        session_id: String,
        /// User ID
        user_id: Uuid,
    },

    /// Thread within a channel
    Thread {
        /// Parent context (guild channel, etc.)
        parent: Box<ContextIdentifier>,
        /// Thread ID
        thread_id: String,
    },

    /// Public broadcast (e.g., Twitter mention, public channel)
    PublicBroadcast {
        /// Platform
        platform: String,
        /// Broadcast ID
        broadcast_id: String,
    },
}

impl ContextIdentifier {
    /// Create a DM context with a single participant.
    #[must_use]
    pub fn dm(user_id: Uuid) -> Self {
        Self::DirectMessage {
            participant_ids: vec![user_id],
        }
    }

    /// Create a guild channel context.
    #[must_use]
    pub fn guild_channel(guild_id: impl Into<String>, channel_id: impl Into<String>) -> Self {
        Self::GuildChannel {
            guild_id: guild_id.into(),
            channel_id: channel_id.into(),
        }
    }

    /// Create a CLI session context.
    #[must_use]
    pub fn cli_session(session_id: impl Into<String>, user_id: Uuid) -> Self {
        Self::CliSession {
            session_id: session_id.into(),
            user_id,
        }
    }

    /// Check if this is a private context (DM, group chat).
    #[must_use]
    pub fn is_private(&self) -> bool {
        matches!(
            self,
            Self::DirectMessage { .. }
                | Self::GroupChat { .. }
                | Self::CliSession { .. }
                | Self::WebSession { .. }
        )
    }

    /// Check if this is a public context.
    #[must_use]
    pub fn is_public(&self) -> bool {
        matches!(self, Self::PublicBroadcast { .. })
    }

    /// Get the participants in this context, if applicable.
    #[must_use]
    pub fn participants(&self) -> Option<&[Uuid]> {
        match self {
            Self::DirectMessage { participant_ids }
            | Self::GroupChat {
                participant_ids, ..
            } => Some(participant_ids),
            Self::WebSession { .. } | Self::CliSession { .. } => {
                // Single user, return as slice would require owned data
                None
            },
            _ => None,
        }
    }

    /// Get the single user for session contexts.
    #[must_use]
    pub fn session_user(&self) -> Option<Uuid> {
        match self {
            Self::WebSession { user_id, .. } | Self::CliSession { user_id, .. } => Some(*user_id),
            Self::DirectMessage { participant_ids } if participant_ids.len() == 1 => {
                Some(participant_ids[0])
            },
            _ => None,
        }
    }
}

impl fmt::Display for ContextIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectMessage { participant_ids } => {
                let ids: Vec<String> = participant_ids
                    .iter()
                    .map(|id| id.to_string()[..8].to_string())
                    .collect();
                write!(f, "dm:{}", ids.join(","))
            },
            Self::GuildChannel {
                guild_id,
                channel_id,
            } => {
                write!(f, "guild:{guild_id}/channel:{channel_id}")
            },
            Self::GroupChat {
                group_id,
                participant_ids,
            } => {
                write!(f, "group:{group_id}({})", participant_ids.len())
            },
            Self::WebSession {
                session_id,
                user_id,
            } => {
                write!(
                    f,
                    "web:{}@{}",
                    &session_id[..8.min(session_id.len())],
                    &user_id.to_string()[..8]
                )
            },
            Self::CliSession {
                session_id,
                user_id,
            } => {
                write!(
                    f,
                    "cli:{}@{}",
                    &session_id[..8.min(session_id.len())],
                    &user_id.to_string()[..8]
                )
            },
            Self::Thread { parent, thread_id } => {
                write!(f, "{parent}/thread:{thread_id}")
            },
            Self::PublicBroadcast {
                platform,
                broadcast_id,
            } => {
                write!(f, "public:{platform}:{broadcast_id}")
            },
        }
    }
}

/// Input classification for security decisions.
///
/// Different input sources have different trust levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputClassification {
    /// Verified user signature - highest trust
    SignedUser,
    /// Pre-authorized via capability token
    Capability,
    /// Tool results, external data - never execute directly
    Untrusted,
    /// From the agent itself (e.g., reasoning)
    AgentInternal,
}

impl InputClassification {
    /// Check if this input can be trusted to make security decisions.
    #[must_use]
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::SignedUser | Self::Capability)
    }

    /// Check if this input should be treated as potentially malicious.
    #[must_use]
    pub fn requires_sanitization(&self) -> bool {
        matches!(self, Self::Untrusted)
    }
}

impl fmt::Display for InputClassification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedUser => write!(f, "signed_user"),
            Self::Capability => write!(f, "capability"),
            Self::Untrusted => write!(f, "untrusted"),
            Self::AgentInternal => write!(f, "agent_internal"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_id_creation() {
        let msg = MessageId::new("discord", "123456789");
        assert_eq!(msg.frontend, "discord");
        assert_eq!(msg.id, "123456789");
        assert_eq!(msg.to_string(), "discord:123456789");
    }

    #[test]
    fn test_message_id_helpers() {
        let cli = MessageId::cli(42);
        assert_eq!(cli.frontend, "cli");
        assert_eq!(cli.id, "seq_42");

        let discord = MessageId::discord(123456789);
        assert_eq!(discord.frontend, "discord");
        assert_eq!(discord.id, "123456789");
    }

    #[test]
    fn test_context_identifier_dm() {
        let user_id = Uuid::new_v4();
        let ctx = ContextIdentifier::dm(user_id);
        assert!(ctx.is_private());
        assert!(!ctx.is_public());
        assert_eq!(ctx.session_user(), Some(user_id));
    }

    #[test]
    fn test_context_identifier_guild() {
        let ctx = ContextIdentifier::guild_channel("guild123", "channel456");
        assert!(!ctx.is_private());
        assert!(!ctx.is_public());

        let display = ctx.to_string();
        assert!(display.contains("guild:guild123"));
        assert!(display.contains("channel:channel456"));
    }

    #[test]
    fn test_tagged_message() {
        let user = AstridUserId::new();
        let msg = TaggedMessage::new(
            MessageId::discord(123),
            user.clone(),
            "discord_user_123",
            FrontendType::Discord,
            ContextIdentifier::dm(user.id),
            "Hello, world!",
        );

        assert!(msg.is_dm());
        assert!(!msg.is_signed());
    }

    #[test]
    fn test_input_classification() {
        assert!(InputClassification::SignedUser.is_trusted());
        assert!(InputClassification::Capability.is_trusted());
        assert!(!InputClassification::Untrusted.is_trusted());
        assert!(InputClassification::Untrusted.requires_sanitization());
    }
}
