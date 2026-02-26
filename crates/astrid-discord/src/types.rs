//! Minimal Discord API types for the WASM capsule.
//!
//! Only the fields we read or write are modelled. Unknown fields are
//! silently dropped by serde's default deserialization.
//!
//! Some types and constants are defined ahead of their Phase 1 usage
//! for Phase 2 (interactive features) support.

use serde::{Deserialize, Serialize};

// ── Interaction Types ────────────────────────────────────────

/// Discord interaction type constants.
pub(crate) mod interaction_type {
    /// Ping (used for endpoint verification).
    pub(crate) const PING: u8 = 1;
    /// Application command (slash command).
    pub(crate) const APPLICATION_COMMAND: u8 = 2;
    /// Message component (button, select menu).
    pub(crate) const MESSAGE_COMPONENT: u8 = 3;
    /// Modal submit.
    pub(crate) const MODAL_SUBMIT: u8 = 5;
}

/// Discord interaction callback type constants.
pub(crate) mod callback_type {
    /// ACK a ping.
    pub(crate) const PONG: u8 = 1;
    /// Respond with a message.
    pub(crate) const CHANNEL_MESSAGE_WITH_SOURCE: u8 = 4;
    /// ACK with deferred response (shows "thinking...").
    pub(crate) const DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE: u8 = 5;
    /// ACK a component interaction (update the message).
    pub(crate) const UPDATE_MESSAGE: u8 = 7;
    /// Respond with a modal popup.
    pub(crate) const MODAL: u8 = 9;
}

/// Component type constants.
pub(crate) mod component_type {
    pub(crate) const ACTION_ROW: u8 = 1;
    pub(crate) const BUTTON: u8 = 2;
    pub(crate) const TEXT_INPUT: u8 = 4;
}

/// Button style constants.
pub(crate) mod button_style {
    pub(crate) const PRIMARY: u8 = 1;
    pub(crate) const SECONDARY: u8 = 2;
    pub(crate) const SUCCESS: u8 = 3;
    pub(crate) const DANGER: u8 = 4;
}

/// Command option type constants.
pub(crate) mod option_type {
    pub(crate) const STRING: u8 = 3;
}

// ── Inbound Types (from Discord) ─────────────────────────────

/// A Discord interaction payload.
#[derive(Debug, Deserialize)]
pub(crate) struct Interaction {
    pub id: String,
    #[serde(rename = "type")]
    pub interaction_type: u8,
    pub token: String,
    #[serde(default)]
    pub data: Option<InteractionData>,
    #[serde(default)]
    pub member: Option<GuildMember>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub message: Option<Message>,
}

impl Interaction {
    /// Extract the user ID from either `member.user` or top-level `user`.
    pub(crate) fn user_id(&self) -> Option<&str> {
        self.member
            .as_ref()
            .and_then(|m| m.user.as_ref())
            .or(self.user.as_ref())
            .map(|u| u.id.as_str())
    }
}

/// Data payload within an interaction.
#[derive(Debug, Deserialize)]
pub(crate) struct InteractionData {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    pub data_type: Option<u8>,
    #[serde(default)]
    pub options: Option<Vec<CommandOption>>,
    #[serde(default)]
    pub custom_id: Option<String>,
    #[serde(default)]
    pub component_type: Option<u8>,
    #[serde(default)]
    pub components: Option<Vec<ModalComponent>>,
}

/// A slash command option value.
#[derive(Debug, Deserialize)]
pub(crate) struct CommandOption {
    pub name: String,
    #[serde(rename = "type")]
    pub option_type: u8,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// A modal component row (for modal submit parsing).
#[derive(Debug, Deserialize)]
pub(crate) struct ModalComponent {
    #[serde(rename = "type")]
    pub component_type: u8,
    #[serde(default)]
    pub components: Option<Vec<ModalTextInput>>,
}

/// A text input value from a modal.
#[derive(Debug, Deserialize)]
pub(crate) struct ModalTextInput {
    pub custom_id: String,
    pub value: String,
}

/// A Discord guild member.
#[derive(Debug, Deserialize)]
pub(crate) struct GuildMember {
    #[serde(default)]
    pub user: Option<User>,
}

/// A Discord user.
#[derive(Debug, Deserialize)]
pub(crate) struct User {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
}

/// A Discord message.
#[derive(Debug, Deserialize)]
pub(crate) struct Message {
    pub id: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

// ── Outbound Types (to Discord) ──────────────────────────────

/// Interaction callback response body.
#[derive(Serialize)]
pub(crate) struct InteractionResponse {
    #[serde(rename = "type")]
    pub response_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<InteractionCallbackData>,
}

/// Data for an interaction callback.
#[derive(Serialize, Default)]
pub(crate) struct InteractionCallbackData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<Embed>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
    /// 64 = ephemeral message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,
}

/// An embed object.
#[derive(Serialize)]
pub(crate) struct Embed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<EmbedField>>,
}

/// An embed field.
#[derive(Serialize)]
pub(crate) struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<bool>,
}

/// A message component (action row, button, text input).
#[derive(Serialize)]
pub(crate) struct Component {
    #[serde(rename = "type")]
    pub component_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

/// Slash command definition for registration.
#[derive(Serialize)]
pub(crate) struct SlashCommandDef {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<CommandOptionDef>,
}

/// Slash command option definition.
#[derive(Serialize)]
pub(crate) struct CommandOptionDef {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub option_type: u8,
    pub required: bool,
}

/// HTTP request payload for the HTTP Airlock.
#[derive(Serialize)]
pub(crate) struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: std::collections::HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// HTTP response from the HTTP Airlock.
#[derive(Deserialize)]
pub(crate) struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub body: Option<String>,
}

/// Webhook message body (for editing deferred responses / followups).
#[derive(Serialize, Default)]
pub(crate) struct WebhookMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<Embed>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Interaction constants ---

    #[test]
    fn interaction_type_constants() {
        assert_eq!(interaction_type::PING, 1);
        assert_eq!(interaction_type::APPLICATION_COMMAND, 2);
        assert_eq!(interaction_type::MESSAGE_COMPONENT, 3);
        assert_eq!(interaction_type::MODAL_SUBMIT, 5);
    }

    #[test]
    fn callback_type_constants() {
        assert_eq!(callback_type::PONG, 1);
        assert_eq!(callback_type::CHANNEL_MESSAGE_WITH_SOURCE, 4);
        assert_eq!(callback_type::DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE, 5);
        assert_eq!(callback_type::UPDATE_MESSAGE, 7);
        assert_eq!(callback_type::MODAL, 9);
    }

    #[test]
    fn component_type_constants() {
        assert_eq!(component_type::ACTION_ROW, 1);
        assert_eq!(component_type::BUTTON, 2);
        assert_eq!(component_type::TEXT_INPUT, 4);
    }

    #[test]
    fn button_style_constants() {
        assert_eq!(button_style::PRIMARY, 1);
        assert_eq!(button_style::SECONDARY, 2);
        assert_eq!(button_style::SUCCESS, 3);
        assert_eq!(button_style::DANGER, 4);
    }

    #[test]
    fn option_type_string_constant() {
        assert_eq!(option_type::STRING, 3);
    }

    // --- Interaction deserialization ---

    #[test]
    fn deserialize_ping_interaction() {
        let json = serde_json::json!({
            "id": "123456",
            "type": 1,
            "token": "ping-token"
        });
        let interaction: Interaction = serde_json::from_value(json).unwrap();
        assert_eq!(interaction.id, "123456");
        assert_eq!(interaction.interaction_type, interaction_type::PING);
        assert_eq!(interaction.token, "ping-token");
        assert!(interaction.data.is_none());
        assert!(interaction.member.is_none());
        assert!(interaction.user.is_none());
        assert!(interaction.channel_id.is_none());
        assert!(interaction.guild_id.is_none());
    }

    #[test]
    fn deserialize_slash_command_interaction() {
        let json = serde_json::json!({
            "id": "cmd-1",
            "type": 2,
            "token": "cmd-token",
            "channel_id": "ch-42",
            "guild_id": "guild-99",
            "data": {
                "id": "data-1",
                "name": "chat",
                "type": 1,
                "options": [{
                    "name": "message",
                    "type": 3,
                    "value": "hello world"
                }]
            },
            "member": {
                "user": {
                    "id": "user-55",
                    "username": "testuser"
                }
            }
        });
        let interaction: Interaction = serde_json::from_value(json).unwrap();
        assert_eq!(
            interaction.interaction_type,
            interaction_type::APPLICATION_COMMAND
        );
        assert_eq!(interaction.channel_id.as_deref(), Some("ch-42"));
        assert_eq!(interaction.guild_id.as_deref(), Some("guild-99"));

        let data = interaction.data.as_ref().unwrap();
        assert_eq!(data.name.as_deref(), Some("chat"));
        let opts = data.options.as_ref().unwrap();
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].name, "message");
        assert_eq!(opts[0].option_type, option_type::STRING);
        assert_eq!(
            opts[0].value.as_ref().unwrap().as_str(),
            Some("hello world")
        );
    }

    #[test]
    fn deserialize_component_interaction() {
        let json = serde_json::json!({
            "id": "btn-1",
            "type": 3,
            "token": "btn-token",
            "data": {
                "custom_id": "apr:req-123:allow_once",
                "component_type": 2
            },
            "user": {
                "id": "user-77"
            }
        });
        let interaction: Interaction = serde_json::from_value(json).unwrap();
        assert_eq!(
            interaction.interaction_type,
            interaction_type::MESSAGE_COMPONENT
        );
        let data = interaction.data.as_ref().unwrap();
        assert_eq!(data.custom_id.as_deref(), Some("apr:req-123:allow_once"));
        assert_eq!(data.component_type, Some(component_type::BUTTON));
    }

    #[test]
    fn deserialize_ignores_unknown_fields() {
        let json = serde_json::json!({
            "id": "x",
            "type": 1,
            "token": "t",
            "unknown_field": "should be ignored",
            "another_unknown": 42
        });
        let interaction: Interaction = serde_json::from_value(json).unwrap();
        assert_eq!(interaction.id, "x");
    }

    // --- Interaction::user_id ---

    #[test]
    fn user_id_from_member() {
        let interaction = Interaction {
            id: "1".into(),
            interaction_type: 2,
            token: "t".into(),
            data: None,
            member: Some(GuildMember {
                user: Some(User {
                    id: "member-user".into(),
                    username: None,
                }),
            }),
            user: Some(User {
                id: "direct-user".into(),
                username: None,
            }),
            channel_id: None,
            guild_id: None,
            message: None,
        };
        // member.user takes priority.
        assert_eq!(interaction.user_id(), Some("member-user"));
    }

    #[test]
    fn user_id_from_top_level_user() {
        let interaction = Interaction {
            id: "1".into(),
            interaction_type: 2,
            token: "t".into(),
            data: None,
            member: None,
            user: Some(User {
                id: "dm-user".into(),
                username: None,
            }),
            channel_id: None,
            guild_id: None,
            message: None,
        };
        assert_eq!(interaction.user_id(), Some("dm-user"));
    }

    #[test]
    fn user_id_none_when_no_user() {
        let interaction = Interaction {
            id: "1".into(),
            interaction_type: 2,
            token: "t".into(),
            data: None,
            member: None,
            user: None,
            channel_id: None,
            guild_id: None,
            message: None,
        };
        assert!(interaction.user_id().is_none());
    }

    #[test]
    fn user_id_none_when_member_has_no_user() {
        let interaction = Interaction {
            id: "1".into(),
            interaction_type: 2,
            token: "t".into(),
            data: None,
            member: Some(GuildMember { user: None }),
            user: None,
            channel_id: None,
            guild_id: None,
            message: None,
        };
        assert!(interaction.user_id().is_none());
    }

    // --- Outbound type serialization ---

    #[test]
    fn interaction_response_serializes_pong() {
        let resp = InteractionResponse {
            response_type: callback_type::PONG,
            data: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["type"], 1);
        assert!(json.get("data").is_none());
    }

    #[test]
    fn interaction_response_serializes_with_data() {
        let resp = InteractionResponse {
            response_type: callback_type::CHANNEL_MESSAGE_WITH_SOURCE,
            data: Some(InteractionCallbackData {
                content: Some("Hello!".to_string()),
                flags: Some(64),
                ..Default::default()
            }),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["type"], 4);
        assert_eq!(json["data"]["content"], "Hello!");
        assert_eq!(json["data"]["flags"], 64);
        // Optional None fields should be omitted.
        assert!(json["data"].get("embeds").is_none());
        assert!(json["data"].get("components").is_none());
    }

    #[test]
    fn callback_data_default_has_all_none() {
        let data = InteractionCallbackData::default();
        let json = serde_json::to_value(&data).unwrap();
        // All fields should be skipped (empty object).
        assert_eq!(json, serde_json::json!({}));
    }

    #[test]
    fn embed_serializes_with_color() {
        let embed = Embed {
            title: Some("Error".into()),
            description: Some("Something went wrong".into()),
            color: Some(0xE74C3C),
            fields: None,
        };
        let json = serde_json::to_value(&embed).unwrap();
        assert_eq!(json["title"], "Error");
        assert_eq!(json["description"], "Something went wrong");
        assert_eq!(json["color"], 0xE74C3C);
        assert!(json.get("fields").is_none());
    }

    #[test]
    fn embed_with_fields() {
        let embed = Embed {
            title: None,
            description: None,
            color: None,
            fields: Some(vec![
                EmbedField {
                    name: "Status".into(),
                    value: "Active".into(),
                    inline: Some(true),
                },
                EmbedField {
                    name: "Turn".into(),
                    value: "idle".into(),
                    inline: None,
                },
            ]),
        };
        let json = serde_json::to_value(&embed).unwrap();
        let fields = json["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0]["name"], "Status");
        assert_eq!(fields[0]["inline"], true);
        assert!(fields[1].get("inline").is_none());
    }

    #[test]
    fn component_button_serializes() {
        let btn = Component {
            component_type: component_type::BUTTON,
            components: None,
            style: Some(button_style::SUCCESS),
            label: Some("Approve".into()),
            custom_id: Some("apr:req-1:allow_once".into()),
            disabled: None,
        };
        let json = serde_json::to_value(&btn).unwrap();
        assert_eq!(json["type"], 2);
        assert_eq!(json["style"], 3);
        assert_eq!(json["label"], "Approve");
        assert_eq!(json["custom_id"], "apr:req-1:allow_once");
    }

    #[test]
    fn action_row_with_buttons() {
        let row = Component {
            component_type: component_type::ACTION_ROW,
            components: Some(vec![
                Component {
                    component_type: component_type::BUTTON,
                    components: None,
                    style: Some(button_style::SUCCESS),
                    label: Some("Yes".into()),
                    custom_id: Some("yes".into()),
                    disabled: None,
                },
                Component {
                    component_type: component_type::BUTTON,
                    components: None,
                    style: Some(button_style::DANGER),
                    label: Some("No".into()),
                    custom_id: Some("no".into()),
                    disabled: Some(true),
                },
            ]),
            style: None,
            label: None,
            custom_id: None,
            disabled: None,
        };
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["type"], 1);
        let buttons = json["components"].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[1]["disabled"], true);
    }

    #[test]
    fn slash_command_def_serializes() {
        let cmd = SlashCommandDef {
            name: "chat".into(),
            description: "Talk to AI".into(),
            options: vec![CommandOptionDef {
                name: "message".into(),
                description: "Your message".into(),
                option_type: option_type::STRING,
                required: true,
            }],
        };
        let json = serde_json::to_value(&cmd).unwrap();
        assert_eq!(json["name"], "chat");
        let opts = json["options"].as_array().unwrap();
        assert_eq!(opts[0]["type"], 3);
        assert_eq!(opts[0]["required"], true);
    }

    #[test]
    fn slash_command_def_empty_options_omitted() {
        let cmd = SlashCommandDef {
            name: "help".into(),
            description: "Show help".into(),
            options: vec![],
        };
        let json = serde_json::to_value(&cmd).unwrap();
        // Empty vec should be skipped due to skip_serializing_if.
        assert!(json.get("options").is_none());
    }

    #[test]
    fn webhook_message_default_serializes_empty() {
        let msg = WebhookMessage::default();
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json, serde_json::json!({}));
    }

    #[test]
    fn webhook_message_with_content() {
        let msg = WebhookMessage {
            content: Some("Hello, world!".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["content"], "Hello, world!");
        assert!(json.get("embeds").is_none());
    }

    #[test]
    fn http_request_serializes() {
        let req = HttpRequest {
            method: "POST".into(),
            url: "https://discord.com/api/v10/channels/123/messages".into(),
            headers: [
                ("Authorization".into(), "Bot token123".into()),
                ("Content-Type".into(), "application/json".into()),
            ]
            .into_iter()
            .collect(),
            body: Some(r#"{"content":"hi"}"#.into()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["method"], "POST");
        assert_eq!(json["headers"]["Authorization"], "Bot token123");
    }

    #[test]
    fn http_response_deserializes() {
        let json = serde_json::json!({
            "status": 200,
            "body": "{\"id\":\"msg-1\"}"
        });
        let resp: HttpResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body.as_deref(), Some("{\"id\":\"msg-1\"}"));
    }

    #[test]
    fn http_response_no_body() {
        let json = serde_json::json!({ "status": 204 });
        let resp: HttpResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.status, 204);
        assert!(resp.body.is_none());
    }

    #[test]
    fn modal_submit_deserialization() {
        let json = serde_json::json!({
            "id": "modal-1",
            "type": 5,
            "token": "modal-token",
            "data": {
                "custom_id": "eli:req-5:response",
                "components": [{
                    "type": 1,
                    "components": [{
                        "custom_id": "input-1",
                        "value": "user typed this"
                    }]
                }]
            }
        });
        let interaction: Interaction = serde_json::from_value(json).unwrap();
        assert_eq!(interaction.interaction_type, interaction_type::MODAL_SUBMIT);

        let data = interaction.data.unwrap();
        assert_eq!(data.custom_id.as_deref(), Some("eli:req-5:response"));

        let components = data.components.unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, component_type::ACTION_ROW);

        let inputs = components[0].components.as_ref().unwrap();
        assert_eq!(inputs[0].custom_id, "input-1");
        assert_eq!(inputs[0].value, "user typed this");
    }

    #[test]
    fn message_deserialization() {
        let json = serde_json::json!({
            "id": "msg-42",
            "channel_id": "ch-7",
            "content": "Hello there"
        });
        let msg: Message = serde_json::from_value(json).unwrap();
        assert_eq!(msg.id, "msg-42");
        assert_eq!(msg.channel_id.as_deref(), Some("ch-7"));
        assert_eq!(msg.content.as_deref(), Some("Hello there"));
    }
}
