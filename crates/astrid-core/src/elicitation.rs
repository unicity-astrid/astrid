//! Elicitation types for MCP server-initiated user input requests.
//!
//! These types implement the elicitation protocol where an MCP server
//! can request structured input from the user (text, secrets, selections,
//! confirmations) or redirect them to an external URL flow (OAuth, payments).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

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
