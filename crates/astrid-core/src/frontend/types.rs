use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

use crate::input::ContextIdentifier;
use crate::types::{RiskLevel, SessionId};

/// Current interaction context from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendContext {
    /// Context identifier (channel/DM/session)
    pub context_id: ContextIdentifier,
    /// Current user information
    pub user: FrontendUser,
    /// Channel information
    pub channel: ChannelInfo,
    /// Session information
    pub session: FrontendSessionInfo,
}
impl FrontendContext {
    /// Create a new frontend context.
    #[must_use]
    pub fn new(
        context_id: ContextIdentifier,
        user: FrontendUser,
        channel: ChannelInfo,
        session: FrontendSessionInfo,
    ) -> Self {
        Self {
            context_id,
            user,
            channel,
            session,
        }
    }

    /// Check if this is a DM context.
    #[must_use]
    pub fn is_dm(&self) -> bool {
        matches!(self.context_id, ContextIdentifier::DirectMessage { .. })
    }

    /// Check if this is a private context.
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.context_id.is_private()
    }
}
/// Information about the current user from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendUser {
    /// Frontend-specific user ID
    pub frontend_user_id: String,
    /// Astrid identity (if resolved)
    pub astrid_id: Option<Uuid>,
    /// Display name
    pub display_name: Option<String>,
    /// Whether the user is an admin on this frontend
    pub is_admin: bool,
}
impl FrontendUser {
    /// Create a new frontend user.
    #[must_use]
    pub fn new(frontend_user_id: impl Into<String>) -> Self {
        Self {
            frontend_user_id: frontend_user_id.into(),
            astrid_id: None,
            display_name: None,
            is_admin: false,
        }
    }

    /// Set the Astrid identity.
    #[must_use]
    pub fn with_astrid_id(mut self, id: Uuid) -> Self {
        self.astrid_id = Some(id);
        self
    }

    /// Set the display name.
    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Set admin status.
    #[must_use]
    pub fn with_admin(mut self, is_admin: bool) -> Self {
        self.is_admin = is_admin;
        self
    }
}
/// Information about the current channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelInfo {
    /// Channel ID
    pub id: String,
    /// Channel name
    pub name: Option<String>,
    /// Channel type
    pub channel_type: ChannelType,
    /// Guild/server ID (if applicable)
    pub guild_id: Option<String>,
}
impl ChannelInfo {
    /// Create a DM channel.
    #[must_use]
    pub fn dm(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            channel_type: ChannelType::DirectMessage,
            guild_id: None,
        }
    }

    /// Create a guild channel.
    #[must_use]
    pub fn guild_channel(
        id: impl Into<String>,
        name: impl Into<String>,
        guild_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: Some(name.into()),
            channel_type: ChannelType::GuildText,
            guild_id: Some(guild_id.into()),
        }
    }
}
/// Session information from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendSessionInfo {
    /// Session ID
    pub session_id: SessionId,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// Session metadata
    pub metadata: HashMap<String, String>,
}
impl FrontendSessionInfo {
    /// Create a new session.
    #[must_use]
    pub fn new() -> Self {
        Self {
            session_id: SessionId::new(),
            started_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    /// Add metadata to the session.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}
impl Default for FrontendSessionInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Type of channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    /// Direct message
    DirectMessage,
    /// Guild text channel
    GuildText,
    /// Guild voice channel
    GuildVoice,
    /// Group DM
    GroupDm,
    /// Thread
    Thread,
    /// CLI session
    Cli,
    /// Web session
    Web,
}

impl fmt::Display for ChannelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectMessage => write!(f, "dm"),
            Self::GuildText => write!(f, "text"),
            Self::GuildVoice => write!(f, "voice"),
            Self::GroupDm => write!(f, "group_dm"),
            Self::Thread => write!(f, "thread"),
            Self::Cli => write!(f, "cli"),
            Self::Web => write!(f, "web"),
        }
    }
}

/// MCP elicitation request - server asking for user input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationRequest {
    /// Unique request ID
    pub request_id: Uuid,
    /// Server that is requesting the elicitation
    pub server_name: String,
    /// Schema describing what input is needed
    pub schema: ElicitationSchema,
    /// Human-readable message
    pub message: String,
    /// Whether this is required or optional
    pub required: bool,
}

impl ElicitationRequest {
    /// Create a new elicitation request.
    #[must_use]
    pub fn new(server_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            server_name: server_name.into(),
            schema: ElicitationSchema::Text {
                placeholder: None,
                max_length: None,
            },
            message: message.into(),
            required: true,
        }
    }

    /// Set the schema.
    #[must_use]
    pub fn with_schema(mut self, schema: ElicitationSchema) -> Self {
        self.schema = schema;
        self
    }

    /// Set as optional.
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
}

/// Schema for elicitation input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElicitationSchema {
    /// Free-form text input
    Text {
        /// Placeholder text
        placeholder: Option<String>,
        /// Maximum length
        max_length: Option<usize>,
    },
    /// Password/secret input (masked)
    Secret {
        /// Placeholder text
        placeholder: Option<String>,
    },
    /// Selection from options
    Select {
        /// Available options
        options: Vec<SelectOption>,
        /// Allow multiple selection
        multiple: bool,
    },
    /// Boolean choice
    Confirm {
        /// Default value
        default: bool,
    },
}

/// Option for select schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    /// Value to submit
    pub value: String,
    /// Display label
    pub label: String,
    /// Description
    pub description: Option<String>,
}

impl SelectOption {
    /// Create a new select option.
    #[must_use]
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: None,
        }
    }

    /// Add a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Response to an elicitation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationResponse {
    /// Request ID this responds to
    pub request_id: Uuid,
    /// The action taken
    pub action: ElicitationAction,
}

impl ElicitationResponse {
    /// Create a submit response.
    #[must_use]
    pub fn submit(request_id: Uuid, value: serde_json::Value) -> Self {
        Self {
            request_id,
            action: ElicitationAction::Submit { value },
        }
    }

    /// Create a cancel response.
    #[must_use]
    pub fn cancel(request_id: Uuid) -> Self {
        Self {
            request_id,
            action: ElicitationAction::Cancel,
        }
    }

    /// Create a dismiss response.
    #[must_use]
    pub fn dismiss(request_id: Uuid) -> Self {
        Self {
            request_id,
            action: ElicitationAction::Dismiss,
        }
    }
}

/// Action taken in response to elicitation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElicitationAction {
    /// User submitted a value
    Submit {
        /// The submitted value
        value: serde_json::Value,
    },
    /// User cancelled
    Cancel,
    /// User dismissed (optional elicitation)
    Dismiss,
}

/// URL-mode elicitation for OAuth, payments, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlElicitationRequest {
    /// Unique request ID
    pub request_id: Uuid,
    /// Server that is requesting
    pub server_name: String,
    /// URL to present to the user
    pub url: String,
    /// Human-readable message
    pub message: String,
    /// Type of URL elicitation
    pub elicitation_type: UrlElicitationType,
}

impl UrlElicitationRequest {
    /// Create a new URL elicitation request.
    #[must_use]
    pub fn new(
        server_name: impl Into<String>,
        url: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            server_name: server_name.into(),
            url: url.into(),
            message: message.into(),
            elicitation_type: UrlElicitationType::OAuth,
        }
    }

    /// Set the elicitation type.
    #[must_use]
    pub fn with_type(mut self, elicitation_type: UrlElicitationType) -> Self {
        self.elicitation_type = elicitation_type;
        self
    }
}

/// Type of URL elicitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlElicitationType {
    /// OAuth authentication flow
    OAuth,
    /// Payment flow
    Payment,
    /// Credential collection
    Credentials,
    /// Generic external action
    External,
}

/// Response to a URL elicitation flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlElicitationResponse {
    /// Request ID this responds to.
    pub request_id: Uuid,
    /// Whether the user completed the flow.
    pub completed: bool,
    /// Callback data from the flow (e.g., OAuth authorization code).
    pub callback_data: Option<HashMap<String, String>>,
    /// Error if the flow failed.
    pub error: Option<String>,
}

impl UrlElicitationResponse {
    /// Create a successful response (user completed the flow).
    #[must_use]
    pub fn completed(request_id: Uuid) -> Self {
        Self {
            request_id,
            completed: true,
            callback_data: None,
            error: None,
        }
    }

    /// Create a response indicating the user did not complete the flow.
    #[must_use]
    pub fn not_completed(request_id: Uuid) -> Self {
        Self {
            request_id,
            completed: false,
            callback_data: None,
            error: None,
        }
    }

    /// Attach callback data (e.g., OAuth code).
    #[must_use]
    pub fn with_callback_data(mut self, data: HashMap<String, String>) -> Self {
        self.callback_data = Some(data);
        self
    }
}

/// Request for user approval of an operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Unique request ID
    pub request_id: Uuid,
    /// Operation being requested
    pub operation: String,
    /// Human-readable description
    pub description: String,
    /// Risk level
    pub risk_level: RiskLevel,
    /// Resource being accessed (if applicable)
    pub resource: Option<String>,
    /// Suggested options
    pub options: Vec<ApprovalOption>,
}

impl ApprovalRequest {
    /// Create a new approval request.
    #[must_use]
    pub fn new(operation: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            operation: operation.into(),
            description: description.into(),
            risk_level: RiskLevel::Medium,
            resource: None,
            options: vec![
                ApprovalOption::AllowOnce,
                ApprovalOption::AllowSession,
                ApprovalOption::AllowWorkspace,
                ApprovalOption::AllowAlways,
                ApprovalOption::Deny,
            ],
        }
    }

    /// Set the risk level.
    #[must_use]
    pub fn with_risk_level(mut self, level: RiskLevel) -> Self {
        self.risk_level = level;
        self
    }

    /// Set the resource.
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Set custom options.
    #[must_use]
    pub fn with_options(mut self, options: Vec<ApprovalOption>) -> Self {
        self.options = options;
        self
    }
}

/// Available approval options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOption {
    /// Allow this one time
    AllowOnce,
    /// Allow for the current session
    AllowSession,
    /// Allow for the current workspace (persists in workspace state.db)
    AllowWorkspace,
    /// Allow always (creates capability token)
    AllowAlways,
    /// Deny the operation
    Deny,
}

impl fmt::Display for ApprovalOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllowOnce => write!(f, "Allow Once"),
            Self::AllowSession => write!(f, "Allow Session"),
            Self::AllowWorkspace => write!(f, "Allow Workspace"),
            Self::AllowAlways => write!(f, "Allow Always"),
            Self::Deny => write!(f, "Deny"),
        }
    }
}

/// User's decision on an approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// Request ID this responds to
    pub request_id: Uuid,
    /// The option selected
    pub decision: ApprovalOption,
    /// When the decision was made
    pub decided_at: DateTime<Utc>,
    /// Optional reason provided by user
    pub reason: Option<String>,
}

impl ApprovalDecision {
    /// Create a new approval decision.
    #[must_use]
    pub fn new(request_id: Uuid, decision: ApprovalOption) -> Self {
        Self {
            request_id,
            decision,
            decided_at: Utc::now(),
            reason: None,
        }
    }

    /// Add a reason.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Check if this is an approval (not denial).
    #[must_use]
    pub fn is_approved(&self) -> bool {
        !matches!(self.decision, ApprovalOption::Deny)
    }

    /// Check if this creates a persistent capability token.
    #[must_use]
    pub fn creates_capability(&self) -> bool {
        matches!(self.decision, ApprovalOption::AllowAlways)
    }

    /// Check if this creates a workspace-scoped allowance.
    #[must_use]
    pub fn creates_workspace_allowance(&self) -> bool {
        matches!(self.decision, ApprovalOption::AllowWorkspace)
    }
}

/// Input received from the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInput {
    /// The input content
    pub content: String,
    /// When received
    pub received_at: DateTime<Utc>,
    /// Attachments (file paths, URLs, etc.)
    pub attachments: Vec<Attachment>,
}

impl UserInput {
    /// Create a new user input.
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            received_at: Utc::now(),
            attachments: Vec::new(),
        }
    }

    /// Add an attachment.
    #[must_use]
    pub fn with_attachment(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }
}

/// Attachment to user input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// Attachment type
    pub attachment_type: AttachmentType,
    /// URL or path
    pub location: String,
    /// Optional filename
    pub filename: Option<String>,
    /// MIME type if known
    pub mime_type: Option<String>,
}

/// Type of attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentType {
    /// Local file
    File,
    /// Remote URL
    Url,
    /// Inline data (base64)
    Inline,
}
