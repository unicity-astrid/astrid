//! Discord REST API client via the HTTP Airlock.
//!
//! All Discord API calls are made through `http::request_bytes()`,
//! which routes through the host's SSRF-protected HTTP client.

use std::collections::HashMap;

use astrid_sdk::prelude::*;

use crate::types::{
    CommandOptionDef, HttpRequest, HttpResponse, InteractionCallbackData, InteractionResponse,
    SlashCommandDef, WebhookMessage,
};

/// Base URL for the Discord REST API.
const API_BASE: &str = "https://discord.com/api/v10";

/// Thin wrapper around the Discord REST API.
pub(crate) struct DiscordApi {
    bot_token: String,
    application_id: String,
}

impl DiscordApi {
    /// Create a new API client from explicit credentials.
    pub(crate) fn new(bot_token: String, application_id: String) -> Self {
        Self {
            bot_token,
            application_id,
        }
    }

    /// Create an API client by reading credentials from Sys config.
    pub(crate) fn from_config() -> Result<Self, SysError> {
        let bot_token = sys::get_config_string("DISCORD_BOT_TOKEN")?;
        let app_id = sys::get_config_string("DISCORD_APPLICATION_ID")?;
        if bot_token.is_empty() {
            return Err(SysError::ApiError(
                "DISCORD_BOT_TOKEN not configured".into(),
            ));
        }
        if app_id.is_empty() {
            return Err(SysError::ApiError(
                "DISCORD_APPLICATION_ID not configured".into(),
            ));
        }
        Ok(Self::new(bot_token, app_id))
    }

    /// Return the application ID.
    #[allow(dead_code)] // Used by Phase 2 interaction flows.
    pub(crate) fn application_id(&self) -> &str {
        &self.application_id
    }

    // ── Message Operations ───────────────────────────────────

    /// Send a message to a channel.
    pub(crate) fn send_message(
        &self,
        channel_id: &str,
        content: &str,
    ) -> Result<serde_json::Value, SysError> {
        let body = serde_json::json!({ "content": content });
        self.post(&format!("/channels/{channel_id}/messages"), &body)
    }

    /// Edit an existing message.
    pub(crate) fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<serde_json::Value, SysError> {
        let body = serde_json::json!({ "content": content });
        self.patch(
            &format!("/channels/{channel_id}/messages/{message_id}"),
            &body,
        )
    }

    // ── Interaction Operations ───────────────────────────────

    /// Respond to an interaction (initial callback).
    pub(crate) fn interaction_respond(
        &self,
        interaction_id: &str,
        interaction_token: &str,
        response: &InteractionResponse,
    ) -> Result<(), SysError> {
        let body = serde_json::to_value(response)?;
        let _ = self.post(
            &format!("/interactions/{interaction_id}/{interaction_token}/callback"),
            &body,
        )?;
        Ok(())
    }

    /// Send a deferred acknowledgement (shows "thinking...").
    pub(crate) fn interaction_defer(
        &self,
        interaction_id: &str,
        interaction_token: &str,
    ) -> Result<(), SysError> {
        let response = InteractionResponse {
            response_type: crate::types::callback_type::DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE,
            data: None,
        };
        self.interaction_respond(interaction_id, interaction_token, &response)
    }

    /// Respond with an ephemeral message.
    pub(crate) fn interaction_respond_ephemeral(
        &self,
        interaction_id: &str,
        interaction_token: &str,
        content: &str,
    ) -> Result<(), SysError> {
        let response = InteractionResponse {
            response_type: crate::types::callback_type::CHANNEL_MESSAGE_WITH_SOURCE,
            data: Some(InteractionCallbackData {
                content: Some(content.to_string()),
                flags: Some(64), // EPHEMERAL
                ..Default::default()
            }),
        };
        self.interaction_respond(interaction_id, interaction_token, &response)
    }

    /// Edit the original deferred response.
    pub(crate) fn interaction_edit_original(
        &self,
        interaction_token: &str,
        message: &WebhookMessage,
    ) -> Result<serde_json::Value, SysError> {
        let body = serde_json::to_value(message)?;
        self.patch(
            &format!(
                "/webhooks/{}/{interaction_token}/messages/@original",
                self.application_id
            ),
            &body,
        )
    }

    /// Send a followup message.
    pub(crate) fn interaction_followup(
        &self,
        interaction_token: &str,
        message: &WebhookMessage,
    ) -> Result<serde_json::Value, SysError> {
        let body = serde_json::to_value(message)?;
        self.post(
            &format!("/webhooks/{}/{interaction_token}", self.application_id),
            &body,
        )
    }

    // ── Slash Command Registration ───────────────────────────

    /// Bulk overwrite global application commands.
    pub(crate) fn register_commands(
        &self,
        commands: &[SlashCommandDef],
    ) -> Result<serde_json::Value, SysError> {
        let body = serde_json::to_value(commands)?;
        self.put(
            &format!("/applications/{}/commands", self.application_id),
            &body,
        )
    }

    // ── HTTP Helpers ─────────────────────────────────────────

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, SysError> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bot {}", self.bot_token),
        );
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let req = HttpRequest {
            method: method.to_string(),
            url: format!("{API_BASE}{path}"),
            headers,
            body: body.map(|b| b.to_string()),
        };

        let req_bytes = serde_json::to_vec(&req)?;
        let resp_bytes = http::request_bytes(&req_bytes)?;
        let resp: HttpResponse = serde_json::from_slice(&resp_bytes)?;

        if resp.status >= 400 {
            let detail = resp.body.unwrap_or_default();
            return Err(SysError::ApiError(format!(
                "Discord API error {}: {detail}",
                resp.status
            )));
        }

        match resp.body {
            Some(body_str) if !body_str.is_empty() => {
                serde_json::from_str(&body_str).map_err(Into::into)
            },
            _ => Ok(serde_json::Value::Null),
        }
    }

    fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, SysError> {
        self.request("POST", path, Some(body))
    }

    fn patch(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, SysError> {
        self.request("PATCH", path, Some(body))
    }

    fn put(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, SysError> {
        self.request("PUT", path, Some(body))
    }
}

/// API base URL (exposed for tests).
#[cfg(test)]
pub(crate) fn api_base() -> &'static str {
    API_BASE
}

/// Default slash commands for the Discord bot.
pub(crate) fn default_commands() -> Vec<SlashCommandDef> {
    vec![
        SlashCommandDef {
            name: "chat".into(),
            description: "Send a message to the AI agent".into(),
            options: vec![CommandOptionDef {
                name: "message".into(),
                description: "Your message".into(),
                option_type: crate::types::option_type::STRING,
                required: true,
            }],
        },
        SlashCommandDef {
            name: "reset".into(),
            description: "Reset the current session".into(),
            options: vec![],
        },
        SlashCommandDef {
            name: "status".into(),
            description: "Show agent status".into(),
            options: vec![],
        },
        SlashCommandDef {
            name: "cancel".into(),
            description: "Cancel the current agent turn".into(),
            options: vec![],
        },
        SlashCommandDef {
            name: "help".into(),
            description: "Show help information".into(),
            options: vec![],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_is_v10() {
        assert_eq!(api_base(), "https://discord.com/api/v10");
    }

    #[test]
    fn discord_api_new_stores_credentials() {
        let api = DiscordApi::new("token123".into(), "app456".into());
        assert_eq!(api.application_id(), "app456");
    }

    // --- default_commands ---

    #[test]
    fn default_commands_count() {
        let cmds = default_commands();
        assert_eq!(cmds.len(), 5);
    }

    #[test]
    fn default_commands_names() {
        let cmds = default_commands();
        let names: Vec<&str> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"chat"));
        assert!(names.contains(&"reset"));
        assert!(names.contains(&"status"));
        assert!(names.contains(&"cancel"));
        assert!(names.contains(&"help"));
    }

    #[test]
    fn chat_command_has_required_message_option() {
        let cmds = default_commands();
        let chat = cmds.iter().find(|c| c.name == "chat").unwrap();
        assert_eq!(chat.options.len(), 1);
        let opt = &chat.options[0];
        assert_eq!(opt.name, "message");
        assert_eq!(opt.option_type, crate::types::option_type::STRING);
        assert!(opt.required);
    }

    #[test]
    fn non_chat_commands_have_no_options() {
        let cmds = default_commands();
        for cmd in &cmds {
            if cmd.name != "chat" {
                assert!(
                    cmd.options.is_empty(),
                    "Command '{}' should have no options",
                    cmd.name
                );
            }
        }
    }

    #[test]
    fn default_commands_serializes_to_valid_json() {
        let cmds = default_commands();
        let json = serde_json::to_value(&cmds).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 5);

        // Chat command should have options array.
        let chat = arr.iter().find(|c| c["name"] == "chat").unwrap();
        assert!(chat["options"].is_array());
        assert_eq!(chat["options"].as_array().unwrap().len(), 1);

        // Help command should not have options (skipped).
        let help = arr.iter().find(|c| c["name"] == "help").unwrap();
        assert!(help.get("options").is_none());
    }

    #[test]
    fn command_descriptions_are_nonempty() {
        let cmds = default_commands();
        for cmd in &cmds {
            assert!(
                !cmd.description.is_empty(),
                "Command '{}' has empty description",
                cmd.name
            );
        }
    }

    #[test]
    fn command_names_are_lowercase_alphanumeric() {
        let cmds = default_commands();
        for cmd in &cmds {
            assert!(
                cmd.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "Command name '{}' has invalid chars",
                cmd.name
            );
        }
    }
}
