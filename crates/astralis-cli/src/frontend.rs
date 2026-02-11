//! CLI Frontend implementation.

use astralis_core::frontend::ChannelInfo;
use astralis_core::identity::AstralisUserId;
use astralis_core::input::{ContextIdentifier, MessageId, TaggedMessage};
use astralis_core::verification::{VerificationRequest, VerificationResponse};
use astralis_core::{
    ApprovalDecision, ApprovalOption, ApprovalRequest, ElicitationRequest, ElicitationResponse,
    ElicitationSchema, Frontend, FrontendContext, FrontendSessionInfo, FrontendType, FrontendUser,
    SecurityError, SecurityResult, UrlElicitationRequest, UrlElicitationResponse, UserInput,
};
use async_trait::async_trait;
use dialoguer::{Confirm, Input, Password, Select, theme::ColorfulTheme};
use std::io::{self, Write};

use crate::theme::Theme;

/// CLI frontend implementation.
pub(crate) struct CliFrontend {
    /// Current session info.
    session: FrontendSessionInfo,
    /// User info.
    user: FrontendUser,
}

impl CliFrontend {
    /// Create a new CLI frontend.
    pub(crate) fn new() -> Self {
        Self {
            session: FrontendSessionInfo::new(),
            user: FrontendUser::new("cli_user"),
        }
    }

    /// Set the user.
    #[allow(dead_code)]
    pub(crate) fn with_user(mut self, user: FrontendUser) -> Self {
        self.user = user;
        self
    }

    /// Stream text to stdout (for real-time output).
    #[allow(dead_code, clippy::unused_self)]
    pub(crate) fn stream_text(&self, text: &str) {
        print!("{text}");
        let _ = io::stdout().flush();
    }

    /// Show tool start.
    #[allow(dead_code, clippy::unused_self)]
    pub(crate) fn show_tool_start(&self, tool: &str) {
        println!("\n{}", Theme::info(&format!("Running {tool}...")));
    }

    /// Show tool result.
    #[allow(dead_code, clippy::unused_self)]
    pub(crate) fn show_tool_result(&self, tool: &str, success: bool) {
        if success {
            println!("{}", Theme::success(&format!("{tool} completed")));
        } else {
            println!("{}", Theme::error(&format!("{tool} failed")));
        }
    }
}

impl Default for CliFrontend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Frontend for CliFrontend {
    fn get_context(&self) -> FrontendContext {
        FrontendContext::new(
            ContextIdentifier::cli_session(
                self.session.session_id.0.to_string(),
                uuid::Uuid::nil(),
            ),
            self.user.clone(),
            ChannelInfo {
                id: "cli".to_string(),
                name: Some("CLI".to_string()),
                channel_type: astralis_core::frontend::ChannelType::Cli,
                guild_id: None,
            },
            self.session.clone(),
        )
    }

    async fn elicit(&self, request: ElicitationRequest) -> SecurityResult<ElicitationResponse> {
        println!("\n{}", Theme::separator());
        println!("{}", Theme::header("Input Required"));
        println!("From: {}", request.server_name.cyan());
        println!("{}", request.message);
        println!("{}", Theme::separator());

        let theme = ColorfulTheme::default();

        let value = match request.schema {
            ElicitationSchema::Text {
                placeholder,
                max_length,
            } => {
                let prompt = placeholder.unwrap_or_else(|| "Enter value".to_string());
                let input = Input::<String>::with_theme(&theme)
                    .with_prompt(&prompt)
                    .allow_empty(!request.required);

                let text = input
                    .interact_text()
                    .map_err(|e| SecurityError::McpElicitationFailed(e.to_string()))?;

                if let Some(max) = max_length
                    && text.len() > max
                {
                    return Err(SecurityError::InvalidInput(format!(
                        "Input exceeds max length of {max}"
                    )));
                }

                serde_json::Value::String(text)
            },
            ElicitationSchema::Secret { placeholder } => {
                let prompt = placeholder.unwrap_or_else(|| "Enter secret".to_string());
                let secret = Password::with_theme(&theme)
                    .with_prompt(&prompt)
                    .interact()
                    .map_err(|e| SecurityError::McpElicitationFailed(e.to_string()))?;

                serde_json::Value::String(secret)
            },
            ElicitationSchema::Select { options, multiple } => {
                let labels: Vec<_> = options.iter().map(|o| &o.label).collect();

                if multiple {
                    // Multi-select not directly supported by dialoguer, use checkboxes
                    let selections = dialoguer::MultiSelect::with_theme(&theme)
                        .items(&labels)
                        .interact()
                        .map_err(|e| SecurityError::McpElicitationFailed(e.to_string()))?;

                    let values: Vec<_> = selections
                        .iter()
                        .map(|&i| serde_json::Value::String(options[i].value.clone()))
                        .collect();

                    serde_json::Value::Array(values)
                } else {
                    let selection = Select::with_theme(&theme)
                        .items(&labels)
                        .default(0)
                        .interact()
                        .map_err(|e| SecurityError::McpElicitationFailed(e.to_string()))?;

                    serde_json::Value::String(options[selection].value.clone())
                }
            },
            ElicitationSchema::Confirm { default } => {
                let confirmed = Confirm::with_theme(&theme)
                    .with_prompt("Confirm?")
                    .default(default)
                    .interact()
                    .map_err(|e| SecurityError::McpElicitationFailed(e.to_string()))?;

                serde_json::Value::Bool(confirmed)
            },
        };

        Ok(ElicitationResponse::submit(request.request_id, value))
    }

    async fn elicit_url(
        &self,
        request: UrlElicitationRequest,
    ) -> SecurityResult<UrlElicitationResponse> {
        println!("\n{}", Theme::separator());
        println!("{}", Theme::header("External Authentication Required"));
        println!("{}", request.message);
        println!("\nOpen this URL in your browser:");
        println!("  {}", request.url.underline().blue());

        // Try to open automatically
        if webbrowser::open(&request.url).is_ok() {
            println!("\n{}", Theme::dimmed("(Opened automatically)"));
        }

        // Wait for user confirmation
        let completed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Press Enter when complete")
            .default(true)
            .interact()
            .map_err(|e| SecurityError::McpElicitationFailed(e.to_string()))?;

        println!("{}", Theme::separator());

        if completed {
            Ok(UrlElicitationResponse::completed(request.request_id))
        } else {
            Ok(UrlElicitationResponse::not_completed(request.request_id))
        }
    }

    async fn request_approval(&self, request: ApprovalRequest) -> SecurityResult<ApprovalDecision> {
        println!("\n{}", Theme::separator());
        println!("{}", Theme::header("Approval Required"));
        println!("Action: {}", request.operation.cyan());
        println!("{}", request.description);
        if let Some(ref resource) = request.resource {
            println!("Resource: {resource}");
        }
        println!("Risk: {}", Theme::risk_level(request.risk_level));
        println!("{}", Theme::separator());

        let options: Vec<String> = request.options.iter().map(ToString::to_string).collect();

        let selection = Select::with_theme(&ColorfulTheme::default())
            .items(&options)
            .default(0)
            .interact()
            .map_err(|e| SecurityError::ApprovalDenied {
                reason: e.to_string(),
            })?;

        let decision = request.options[selection];

        let reason = if decision == ApprovalOption::Deny {
            let r = Input::<String>::with_theme(&ColorfulTheme::default())
                .with_prompt("Reason (optional)")
                .allow_empty(true)
                .interact_text()
                .ok();
            r.filter(|s| !s.is_empty())
        } else {
            None
        };

        let mut approval = ApprovalDecision::new(request.request_id, decision);
        if let Some(r) = reason {
            approval = approval.with_reason(r);
        }

        Ok(approval)
    }

    fn show_status(&self, message: &str) {
        // For streaming, just print without newline
        print!("{message}");
        let _ = io::stdout().flush();
    }

    fn show_error(&self, error: &str) {
        eprintln!("{}", Theme::error(error));
    }

    async fn receive_input(&self) -> Option<UserInput> {
        let text = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt(">")
            .allow_empty(false)
            .interact_text()
            .ok()?;

        Some(UserInput::new(text))
    }

    async fn resolve_identity(&self, _frontend_user_id: &str) -> Option<AstralisUserId> {
        // CLI doesn't have persistent identity resolution
        None
    }

    async fn get_message(&self, _message_id: &MessageId) -> Option<TaggedMessage> {
        // CLI doesn't support message fetching
        None
    }

    async fn send_verification(
        &self,
        _user_id: &str,
        request: VerificationRequest,
    ) -> SecurityResult<VerificationResponse> {
        // CLI verification is immediate
        println!("\n{}", Theme::separator());
        println!("{}", Theme::header("Verification Required"));
        println!("{}", request.description);
        println!("{}", Theme::separator());

        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Approve?")
            .default(false)
            .interact()
            .map_err(|e| SecurityError::IdentityVerificationFailed(e.to_string()))?;

        if confirmed {
            Ok(VerificationResponse::confirmed(request.request_id))
        } else {
            Err(SecurityError::VerificationCancelled)
        }
    }

    async fn send_link_code(&self, _user_id: &str, code: &str) -> SecurityResult<()> {
        println!("\n{}", Theme::info(&format!("Link code: {}", code.bold())));
        Ok(())
    }

    fn frontend_type(&self) -> FrontendType {
        FrontendType::Cli
    }
}

use colored::Colorize;
