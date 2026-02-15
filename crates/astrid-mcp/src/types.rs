//! MCP types for tools, resources, and results.

use rmcp::model::{self as rmcp_model, RawContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Definition of an MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name.
    pub name: String,
    /// Server this tool belongs to.
    pub server: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// JSON Schema for input parameters.
    pub input_schema: Value,
}

impl ToolDefinition {
    /// Create a new tool definition.
    #[must_use]
    pub fn new(name: impl Into<String>, server: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            server: server.into(),
            description: None,
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    /// Create from an rmcp `Tool` and server name.
    #[must_use]
    pub fn from_rmcp(tool: &rmcp_model::Tool, server: &str) -> Self {
        Self {
            name: tool.name.to_string(),
            server: server.to_string(),
            description: tool.description.as_deref().map(String::from),
            input_schema: serde_json::to_value(&*tool.input_schema)
                .unwrap_or_else(|_| serde_json::json!({"type": "object"})),
        }
    }

    /// Get the full tool identifier (server:tool).
    #[must_use]
    pub fn full_name(&self) -> String {
        format!("{}:{}", self.server, self.name)
    }

    /// Get the MCP resource URI for this tool.
    #[must_use]
    pub fn resource_uri(&self) -> String {
        format!("mcp://{}:{}", self.server, self.name)
    }
}

/// Result from calling an MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the call succeeded.
    pub success: bool,
    /// Content returned by the tool.
    pub content: Vec<ToolContent>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Whether this result is an error.
    pub is_error: bool,
}

impl ToolResult {
    /// Create a successful result with text content.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: vec![ToolContent::Text {
                text: content.into(),
            }],
            error: None,
            is_error: false,
        }
    }

    /// Create an error result.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self {
            success: false,
            content: vec![ToolContent::Text { text: msg.clone() }],
            error: Some(msg),
            is_error: true,
        }
    }

    /// Get text content as a single string.
    #[must_use]
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl From<rmcp_model::CallToolResult> for ToolResult {
    fn from(result: rmcp_model::CallToolResult) -> Self {
        let is_error = result.is_error.unwrap_or(false);
        let content: Vec<ToolContent> = result.content.iter().map(ToolContent::from_rmcp).collect();

        let error = if is_error {
            // Extract text content as the error message
            let text = content
                .iter()
                .filter_map(|c| match c {
                    ToolContent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                Some("Unknown error".to_string())
            } else {
                Some(text)
            }
        } else {
            None
        };

        Self {
            success: !is_error,
            content,
            error,
            is_error,
        }
    }
}

/// Content types returned by tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    /// Text content.
    Text {
        /// The text.
        text: String,
    },
    /// Image content.
    Image {
        /// Base64-encoded image data.
        data: String,
        /// MIME type.
        mime_type: String,
    },
    /// Resource reference.
    Resource {
        /// Resource URI.
        uri: String,
        /// Resource data.
        data: Option<String>,
        /// MIME type.
        mime_type: Option<String>,
    },
}

impl ToolContent {
    /// Convert from an rmcp `Content` (which is `Annotated<RawContent>`).
    fn from_rmcp(content: &rmcp_model::Content) -> Self {
        match &**content {
            RawContent::Text(text) => Self::Text {
                text: text.text.clone(),
            },
            RawContent::Image(image) => Self::Image {
                data: image.data.clone(),
                mime_type: image.mime_type.clone(),
            },
            RawContent::Resource(embedded) => {
                let (uri, data, mime_type) = match &embedded.resource {
                    rmcp_model::ResourceContents::TextResourceContents {
                        uri,
                        mime_type,
                        text,
                        ..
                    } => (uri.clone(), Some(text.clone()), mime_type.clone()),
                    rmcp_model::ResourceContents::BlobResourceContents {
                        uri,
                        mime_type,
                        blob,
                        ..
                    } => (uri.clone(), Some(blob.clone()), mime_type.clone()),
                };
                Self::Resource {
                    uri,
                    data,
                    mime_type,
                }
            },
            // Audio and ResourceLink variants map to text fallbacks
            RawContent::Audio(_) => Self::Text {
                text: "[audio content]".to_string(),
            },
            RawContent::ResourceLink(resource) => Self::Resource {
                uri: resource.uri.clone(),
                data: None,
                mime_type: resource.mime_type.clone(),
            },
        }
    }
}

/// Definition of an MCP resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDefinition {
    /// Resource URI.
    pub uri: String,
    /// Server this resource belongs to.
    pub server: String,
    /// Human-readable name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// MIME type.
    pub mime_type: Option<String>,
}

impl ResourceDefinition {
    /// Create from an rmcp `Resource` (which is `Annotated<RawResource>`) and server name.
    #[must_use]
    pub fn from_rmcp(resource: &rmcp_model::Resource, server: &str) -> Self {
        Self {
            uri: resource.uri.clone(),
            server: server.to_string(),
            name: resource.name.clone(),
            description: resource.description.clone(),
            mime_type: resource.mime_type.clone(),
        }
    }
}

/// Content returned when reading a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContent {
    /// Resource URI.
    pub uri: String,
    /// Text content (for text resources).
    pub text: Option<String>,
    /// Binary content as base64 (for blob resources).
    pub blob: Option<String>,
    /// MIME type.
    pub mime_type: Option<String>,
}

impl ResourceContent {
    /// Convert from an rmcp `ResourceContents`.
    #[must_use]
    pub fn from_rmcp(contents: &rmcp_model::ResourceContents) -> Self {
        match contents {
            rmcp_model::ResourceContents::TextResourceContents {
                uri,
                mime_type,
                text,
                ..
            } => Self {
                uri: uri.clone(),
                text: Some(text.clone()),
                blob: None,
                mime_type: mime_type.clone(),
            },
            rmcp_model::ResourceContents::BlobResourceContents {
                uri,
                mime_type,
                blob,
                ..
            } => Self {
                uri: uri.clone(),
                text: None,
                blob: Some(blob.clone()),
                mime_type: mime_type.clone(),
            },
        }
    }
}

/// Definition of an MCP prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptDefinition {
    /// Prompt name.
    pub name: String,
    /// Server this prompt belongs to.
    pub server: String,
    /// Description.
    pub description: Option<String>,
    /// Arguments schema.
    pub arguments: Option<Vec<PromptArgument>>,
}

impl PromptDefinition {
    /// Create from an rmcp `Prompt` and server name.
    #[must_use]
    pub fn from_rmcp(prompt: &rmcp_model::Prompt, server: &str) -> Self {
        Self {
            name: prompt.name.clone(),
            server: server.to_string(),
            description: prompt.description.clone(),
            arguments: prompt.arguments.as_ref().map(|args| {
                args.iter()
                    .map(|a| PromptArgument {
                        name: a.name.clone(),
                        description: a.description.clone(),
                        required: a.required.unwrap_or(false),
                    })
                    .collect()
            }),
        }
    }
}

/// Argument for an MCP prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    /// Argument name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Whether required.
    #[serde(default)]
    pub required: bool,
}

/// Content returned from getting a prompt.
#[derive(Debug, Clone)]
pub struct PromptContent {
    /// Description of the prompt.
    pub description: Option<String>,
    /// Rendered prompt messages.
    pub messages: Vec<PromptMessage>,
}

/// A message in a prompt result.
#[derive(Debug, Clone)]
pub struct PromptMessage {
    /// Role of the message sender.
    pub role: String,
    /// Text content of the message.
    pub content: String,
}

impl PromptContent {
    /// Convert from an rmcp `GetPromptResult`.
    #[must_use]
    pub fn from_rmcp(result: &rmcp_model::GetPromptResult) -> Self {
        Self {
            description: result.description.clone(),
            messages: result
                .messages
                .iter()
                .map(|m| {
                    let role = match m.role {
                        rmcp_model::PromptMessageRole::User => "user",
                        rmcp_model::PromptMessageRole::Assistant => "assistant",
                    };
                    let content = match &m.content {
                        rmcp_model::PromptMessageContent::Text { text } => text.clone(),
                        rmcp_model::PromptMessageContent::Image { image } => {
                            format!("[image: {}]", image.mime_type)
                        },
                        rmcp_model::PromptMessageContent::Resource { resource } => {
                            resource.get_text()
                        },
                        rmcp_model::PromptMessageContent::ResourceLink { link } => {
                            format!("[resource: {}]", link.uri)
                        },
                    };
                    PromptMessage {
                        role: role.to_string(),
                        content,
                    }
                })
                .collect(),
        }
    }
}

/// Server capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ServerCapabilities {
    /// Whether the server supports tools.
    #[serde(default)]
    pub tools: bool,
    /// Whether the server supports resources.
    #[serde(default)]
    pub resources: bool,
    /// Whether the server supports prompts.
    #[serde(default)]
    pub prompts: bool,
    /// Whether the server supports sampling.
    #[serde(default)]
    pub sampling: bool,
    /// Whether the server supports elicitation.
    #[serde(default)]
    pub elicitation: bool,
}

impl ServerCapabilities {
    /// Convert from rmcp `ServerCapabilities`.
    #[must_use]
    pub fn from_rmcp(caps: &rmcp_model::ServerCapabilities) -> Self {
        Self {
            tools: caps.tools.is_some(),
            resources: caps.resources.is_some(),
            prompts: caps.prompts.is_some(),
            // Server capabilities don't have sampling/elicitation fields;
            // those are client capabilities. Default to false.
            sampling: false,
            elicitation: false,
        }
    }
}

/// Information about a running server.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,
    /// Protocol version.
    pub protocol_version: String,
    /// Server capabilities.
    pub capabilities: ServerCapabilities,
    /// Server instructions (for LLM).
    pub instructions: Option<String>,
}

impl ServerInfo {
    /// Convert from rmcp `InitializeResult` and a server name.
    #[must_use]
    pub fn from_rmcp(info: &rmcp_model::InitializeResult, name: &str) -> Self {
        Self {
            name: name.to_string(),
            protocol_version: info.protocol_version.to_string(),
            capabilities: ServerCapabilities::from_rmcp(&info.capabilities),
            instructions: info.instructions.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definition() {
        let tool = ToolDefinition::new("read_file", "filesystem");
        assert_eq!(tool.full_name(), "filesystem:read_file");
        assert_eq!(tool.resource_uri(), "mcp://filesystem:read_file");
    }

    #[test]
    fn test_tool_result_text() {
        let result = ToolResult::text("Hello, world!");
        assert!(result.success);
        assert!(!result.is_error);
        assert_eq!(result.text_content(), "Hello, world!");
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("Something went wrong");
        assert!(!result.success);
        assert!(result.is_error);
        assert_eq!(result.error, Some("Something went wrong".to_string()));
    }
}
