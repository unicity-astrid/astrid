//! Discord bot frontend capsule for the Astrid agent runtime.
//!
//! This WASM capsule implements a Discord bot that connects to the
//! Astrid agent runtime via the Airlock syscall boundaries. The
//! host-side `DiscordGatewayProxy` (in `astrid-gateway`) maintains a
//! persistent `WebSocket` connection to Discord's Gateway and relays
//! `MESSAGE_CREATE` / `INTERACTION_CREATE` events to this capsule
//! via IPC.
//!
//! The capsule dispatches on `event.type`: `"interaction"` for slash
//! commands and button presses, `"message"` for regular channel
//! messages. HTTP Interactions (Strategy A) can also deliver events
//! through the same IPC bus as an alternative for public-facing
//! deployments.
//!
//! ## Architecture
//!
//! - **HTTP Airlock**: All Discord REST API calls
//! - **Uplink Airlock**: Routes user messages to the agent runtime
//! - **IPC Airlock**: Receives agent output and Gateway events
//! - **KV Airlock**: Session state persistence
//! - **Sys Airlock**: Logging and configuration
//! - **Cron Airlock**: Periodic polling for events

#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod discord_api;
mod format;
mod session;
#[allow(dead_code)] // Phase 2 types defined ahead of use.
mod types;

use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::discord_api::{DiscordApi, default_commands};
use crate::format::{chunk_discord, sanitize_for_discord};
use crate::session::{
    ActiveTurn, InitState, SessionScope, SessionState, clear_active_turn, get_active_turn,
    get_init_state, get_session, remove_session, set_active_turn, set_init_state, set_session,
};
use crate::types::{
    InteractionCallbackData, InteractionResponse, WebhookMessage, callback_type, interaction_type,
};

// ── Arg Types ────────────────────────────────────────────────

#[derive(Serialize)]
struct ToolOutput {
    content: String,
    is_error: bool,
}

#[derive(Deserialize, Default)]
struct SendArgs {
    channel_id: String,
    content: String,
}

#[derive(Deserialize, Default)]
struct EditArgs {
    channel_id: String,
    message_id: String,
    content: String,
}

#[derive(Deserialize, Default)]
struct EmptyArgs {}

// ── Capsule ──────────────────────────────────────────────────

#[derive(Default)]
pub struct DiscordCapsule;

#[capsule]
impl DiscordCapsule {
    // ── Tools (callable by the LLM agent) ────────────────────

    #[astrid::tool("discord-send")]
    fn handle_send(&self, args: SendArgs) -> Result<ToolOutput, SysError> {
        let api = DiscordApi::from_config()?;
        let chunks = chunk_discord(&sanitize_for_discord(&args.content), 0);

        let mut result = serde_json::Value::Null;
        for chunk in &chunks {
            result = api.send_message(&args.channel_id, chunk)?;
        }

        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    #[astrid::tool("discord-edit")]
    fn handle_edit(&self, args: EditArgs) -> Result<ToolOutput, SysError> {
        let api = DiscordApi::from_config()?;
        let sanitized = sanitize_for_discord(&args.content);
        // Discord edit replaces the full message; truncate to limit.
        let content = if sanitized.len() > 2000 {
            sanitized[..crate::format::floor_char_boundary(&sanitized, 2000)].to_string()
        } else {
            sanitized
        };
        let result = api.edit_message(&args.channel_id, &args.message_id, &content)?;

        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    // ── Cron Handlers ────────────────────────────────────────

    #[astrid::cron("poll-gateway")]
    fn poll_gateway(&self, _args: EmptyArgs) -> Result<serde_json::Value, SysError> {
        let init = match self.ensure_initialized()? {
            Some(init) => init,
            None => return Ok(serde_json::json!({"action": "continue"})),
        };

        // Poll IPC for pending interaction events.
        let events_bytes = ipc::poll_bytes(init.event_handle.as_bytes())?;
        if events_bytes.is_empty() {
            return Ok(serde_json::json!({"action": "continue"}));
        }

        // Events may be a JSON array or single object.
        let events: Vec<serde_json::Value> = match serde_json::from_slice(&events_bytes) {
            Ok(v) => v,
            Err(_) => {
                // Try single event.
                match serde_json::from_slice(&events_bytes) {
                    Ok(single) => vec![single],
                    Err(e) => {
                        sys::log("error", format!("Failed to parse IPC event: {e}"))?;
                        vec![]
                    },
                }
            },
        };

        for event in &events {
            if let Err(e) = self.process_event(event, &init) {
                sys::log("error", format!("Error processing event: {e}"))?;
            }
        }

        Ok(serde_json::json!({"action": "continue"}))
    }

    #[astrid::cron("heartbeat")]
    fn heartbeat(&self, _args: EmptyArgs) -> Result<serde_json::Value, SysError> {
        // Capsule-side heartbeat is a health check (Gateway heartbeats
        // are managed by the host-side DiscordGatewayProxy).
        let _ = get_init_state()?;
        Ok(serde_json::json!({"action": "continue"}))
    }

    // ── Interceptor ──────────────────────────────────────────

    #[astrid::interceptor("run-hook")]
    fn run_hook(&self, _args: EmptyArgs) -> Result<serde_json::Value, SysError> {
        Ok(serde_json::json!({
            "action": "continue",
            "data": null,
        }))
    }
}

// ── Internal Implementation ──────────────────────────────────

impl DiscordCapsule {
    /// Ensure the capsule is initialized (config read, uplink
    /// registered, commands registered). Returns the init state.
    fn ensure_initialized(&self) -> Result<Option<InitState>, SysError> {
        if let Some(state) = get_init_state()? {
            return Ok(Some(state));
        }

        // Verify credentials are configured (read-only check; not persisted).
        let api = match DiscordApi::from_config() {
            Ok(api) => api,
            Err(_) => {
                sys::log("warn", "Discord capsule not configured — skipping init")?;
                return Ok(None);
            },
        };

        // Register uplink connector.
        let connector_bytes = uplink::register("discord", "discord", "chat")?;
        let connector_id = String::from_utf8_lossy(&connector_bytes).to_string();

        // Register slash commands with Discord.
        if let Err(e) = api.register_commands(&default_commands()) {
            sys::log("warn", format!("Failed to register commands: {e}"))?;
        }

        // Subscribe to agent events on the IPC bus.
        let event_handle_bytes = ipc::subscribe("agent.events")?;
        let event_handle = String::from_utf8_lossy(&event_handle_bytes).to_string();

        let state = InitState {
            connector_id,
            event_handle,
        };
        set_init_state(&state)?;

        sys::log("info", "Discord capsule initialized")?;
        Ok(Some(state))
    }

    /// Check whether the given user/guild is authorized.
    ///
    /// Reads `DISCORD_ALLOWED_USERS` and `DISCORD_ALLOWED_GUILDS` from
    /// Sys config. Empty lists mean "allow all".
    fn check_authorization(
        &self,
        interaction: &types::Interaction,
        api: &DiscordApi,
    ) -> Result<bool, SysError> {
        let user_id = interaction.user_id().unwrap_or("");

        // Check user allowlist.
        let allowed_users = sys::get_config_string("DISCORD_ALLOWED_USERS").unwrap_or_default();
        if !allowed_users.is_empty() {
            let allowed: Vec<&str> = allowed_users.split(',').map(str::trim).collect();
            if !allowed.contains(&user_id) {
                api.interaction_respond_ephemeral(
                    &interaction.id,
                    &interaction.token,
                    "You are not authorized to use this bot.",
                )?;
                return Ok(false);
            }
        }

        // Check guild allowlist.
        let allowed_guilds = sys::get_config_string("DISCORD_ALLOWED_GUILDS").unwrap_or_default();
        if !allowed_guilds.is_empty() {
            let guild_id = interaction.guild_id.as_deref().unwrap_or("");
            let allowed: Vec<&str> = allowed_guilds.split(',').map(str::trim).collect();
            if !allowed.contains(&guild_id) {
                api.interaction_respond_ephemeral(
                    &interaction.id,
                    &interaction.token,
                    "This bot is not authorized in this server.",
                )?;
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Process a single event from the IPC bus.
    fn process_event(&self, event: &serde_json::Value, init: &InitState) -> Result<(), SysError> {
        // Determine event type from the payload.
        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "interaction" => {
                self.handle_interaction(event, init)?;
            },
            "message" => {
                self.handle_message_create(event, init)?;
            },
            "text_chunk" => {
                self.handle_text_chunk(event)?;
            },
            "turn_complete" => {
                self.handle_turn_complete(event)?;
            },
            "error" => {
                self.handle_error_event(event, init)?;
            },
            _ => {
                sys::log("debug", format!("Unknown event type: {event_type}"))?;
            },
        }

        Ok(())
    }

    /// Handle an incoming Discord interaction.
    fn handle_interaction(
        &self,
        event: &serde_json::Value,
        init: &InitState,
    ) -> Result<(), SysError> {
        let payload = event
            .get("payload")
            .ok_or_else(|| SysError::ApiError("Missing interaction payload".into()))?;

        let interaction: types::Interaction = serde_json::from_value(payload.clone())?;

        match interaction.interaction_type {
            interaction_type::PING => {
                // ACK the ping.
                let api = DiscordApi::from_config()?;
                api.interaction_respond(
                    &interaction.id,
                    &interaction.token,
                    &InteractionResponse {
                        response_type: callback_type::PONG,
                        data: None,
                    },
                )?;
            },
            interaction_type::APPLICATION_COMMAND => {
                self.handle_slash_command(&interaction, init)?;
            },
            interaction_type::MESSAGE_COMPONENT => {
                self.handle_component_interaction(&interaction, init)?;
            },
            _ => {
                sys::log(
                    "debug",
                    format!(
                        "Unhandled interaction type: {}",
                        interaction.interaction_type
                    ),
                )?;
            },
        }

        Ok(())
    }

    /// Handle a slash command interaction.
    fn handle_slash_command(
        &self,
        interaction: &types::Interaction,
        init: &InitState,
    ) -> Result<(), SysError> {
        let data = interaction
            .data
            .as_ref()
            .ok_or_else(|| SysError::ApiError("Missing interaction data".into()))?;
        let cmd_name = data.name.as_deref().unwrap_or("");

        let api = DiscordApi::from_config()?;

        // Enforce user/guild authorization.
        if !self.check_authorization(interaction, &api)? {
            return Ok(());
        }

        match cmd_name {
            "chat" => {
                self.handle_chat_command(interaction, data, init, &api)?;
            },
            "reset" => {
                self.handle_reset_command(interaction, init, &api)?;
            },
            "status" => {
                self.handle_status_command(interaction, init, &api)?;
            },
            "cancel" => {
                self.handle_cancel_command(interaction, init, &api)?;
            },
            "help" => {
                self.handle_help_command(interaction, &api)?;
            },
            _ => {
                api.interaction_respond_ephemeral(
                    &interaction.id,
                    &interaction.token,
                    &format!("Unknown command: `{cmd_name}`"),
                )?;
            },
        }

        Ok(())
    }

    /// Handle `/chat <message>`.
    fn handle_chat_command(
        &self,
        interaction: &types::Interaction,
        data: &types::InteractionData,
        init: &InitState,
        api: &DiscordApi,
    ) -> Result<(), SysError> {
        // Extract message text from options.
        let message = data
            .options
            .as_ref()
            .and_then(|opts| opts.iter().find(|o| o.name == "message"))
            .and_then(|o| o.value.as_ref())
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if message.is_empty() {
            api.interaction_respond_ephemeral(
                &interaction.id,
                &interaction.token,
                "Please provide a message.",
            )?;
            return Ok(());
        }

        let user_id = interaction.user_id().unwrap_or("unknown");
        let channel_id = interaction.channel_id.as_deref().unwrap_or("unknown");

        // Determine session scope.
        let scope_str = sys::get_config_string("DISCORD_SESSION_SCOPE").unwrap_or_default();
        let scope = SessionScope::from_config(&scope_str);

        let scope_id = match scope {
            SessionScope::Channel => channel_id,
            SessionScope::User => user_id,
        };

        // Check for existing session / turn state.
        let session = get_session(scope, scope_id)?;

        if let Some(ref s) = session
            && s.turn_in_progress
        {
            api.interaction_respond_ephemeral(
                &interaction.id,
                &interaction.token,
                "A turn is already in progress. \
                 Please wait or use `/cancel`.",
            )?;
            return Ok(());
        }

        // Send deferred response (shows "thinking...").
        api.interaction_defer(&interaction.id, &interaction.token)?;

        // Send message to the agent runtime via Uplink.
        let _ = uplink::send_bytes(
            init.connector_id.as_bytes(),
            user_id.as_bytes(),
            message.as_bytes(),
        )?;

        // Update (or create) session state.
        let session_id = session
            .as_ref()
            .map(|s| s.session_id.clone())
            .unwrap_or_else(|| format!("discord-{scope_id}"));

        let state = SessionState {
            session_id,
            connector_id: init.connector_id.clone(),
            last_message_id: None,
            turn_in_progress: true,
            interaction_token: Some(interaction.token.clone()),
        };
        set_session(scope, scope_id, &state)?;

        // Store the active turn metadata so event handlers can
        // resolve the correct session scope.
        set_active_turn(&ActiveTurn {
            scope: scope_str,
            scope_id: scope_id.to_string(),
            buffer: String::new(),
            channel_id: channel_id.to_string(),
        })?;

        Ok(())
    }

    /// Handle `/reset`.
    fn handle_reset_command(
        &self,
        interaction: &types::Interaction,
        _init: &InitState,
        api: &DiscordApi,
    ) -> Result<(), SysError> {
        let channel_id = interaction.channel_id.as_deref().unwrap_or("unknown");
        let user_id = interaction.user_id().unwrap_or("unknown");

        let scope_str = sys::get_config_string("DISCORD_SESSION_SCOPE").unwrap_or_default();
        let scope = SessionScope::from_config(&scope_str);
        let scope_id = match scope {
            SessionScope::Channel => channel_id,
            SessionScope::User => user_id,
        };

        remove_session(scope, scope_id)?;

        api.interaction_respond_ephemeral(
            &interaction.id,
            &interaction.token,
            "Session reset. Start a new conversation \
             with `/chat`.",
        )?;

        Ok(())
    }

    /// Handle `/status`.
    fn handle_status_command(
        &self,
        interaction: &types::Interaction,
        _init: &InitState,
        api: &DiscordApi,
    ) -> Result<(), SysError> {
        let channel_id = interaction.channel_id.as_deref().unwrap_or("unknown");
        let user_id = interaction.user_id().unwrap_or("unknown");

        let scope_str = sys::get_config_string("DISCORD_SESSION_SCOPE").unwrap_or_default();
        let scope = SessionScope::from_config(&scope_str);
        let scope_id = match scope {
            SessionScope::Channel => channel_id,
            SessionScope::User => user_id,
        };

        let session = get_session(scope, scope_id)?;

        let status_msg = match session {
            Some(s) => {
                let turn = if s.turn_in_progress {
                    "in progress"
                } else {
                    "idle"
                };
                format!("**Session**: `{}`\n**Turn**: {turn}", s.session_id)
            },
            None => "No active session. Use `/chat` to start.".to_string(),
        };

        api.interaction_respond_ephemeral(&interaction.id, &interaction.token, &status_msg)?;

        Ok(())
    }

    /// Handle `/cancel`.
    fn handle_cancel_command(
        &self,
        interaction: &types::Interaction,
        _init: &InitState,
        api: &DiscordApi,
    ) -> Result<(), SysError> {
        let channel_id = interaction.channel_id.as_deref().unwrap_or("unknown");
        let user_id = interaction.user_id().unwrap_or("unknown");

        let scope_str = sys::get_config_string("DISCORD_SESSION_SCOPE").unwrap_or_default();
        let scope = SessionScope::from_config(&scope_str);
        let scope_id = match scope {
            SessionScope::Channel => channel_id,
            SessionScope::User => user_id,
        };

        if let Some(mut session) = get_session(scope, scope_id)? {
            if session.turn_in_progress {
                // Publish cancel event on IPC.
                ipc::publish_json(
                    "agent.cancel",
                    &serde_json::json!({
                        "session_id": session.session_id,
                    }),
                )?;

                session.turn_in_progress = false;
                session.interaction_token = None;
                set_session(scope, scope_id, &session)?;

                api.interaction_respond_ephemeral(
                    &interaction.id,
                    &interaction.token,
                    "Turn cancelled.",
                )?;
            } else {
                api.interaction_respond_ephemeral(
                    &interaction.id,
                    &interaction.token,
                    "No turn in progress.",
                )?;
            }
        } else {
            api.interaction_respond_ephemeral(
                &interaction.id,
                &interaction.token,
                "No active session.",
            )?;
        }

        Ok(())
    }

    /// Handle `/help`.
    fn handle_help_command(
        &self,
        interaction: &types::Interaction,
        api: &DiscordApi,
    ) -> Result<(), SysError> {
        let help_text = "\
**Astrid Discord Bot**\n\n\
`/chat <message>` — Send a message to the AI agent\n\
`/reset` — Reset the current session\n\
`/status` — Show session status\n\
`/cancel` — Cancel the current agent turn\n\
`/help` — Show this help message";

        api.interaction_respond_ephemeral(&interaction.id, &interaction.token, help_text)?;

        Ok(())
    }

    /// Handle a component interaction (button press).
    fn handle_component_interaction(
        &self,
        interaction: &types::Interaction,
        _init: &InitState,
    ) -> Result<(), SysError> {
        let api = DiscordApi::from_config()?;

        // Enforce user/guild authorization.
        if !self.check_authorization(interaction, &api)? {
            return Ok(());
        }

        let custom_id = interaction
            .data
            .as_ref()
            .and_then(|d| d.custom_id.as_deref())
            .unwrap_or("");

        // Parse custom_id format: "apr:{request_id}:{option}"
        // or "eli:{request_id}:{option}".
        if custom_id.starts_with("apr:") {
            self.handle_approval_button(interaction, custom_id, &api)?;
        } else if custom_id.starts_with("eli:") {
            self.handle_elicitation_button(interaction, custom_id, &api)?;
        } else {
            api.interaction_respond_ephemeral(
                &interaction.id,
                &interaction.token,
                "Unknown button action.",
            )?;
        }

        Ok(())
    }

    /// Handle an approval button press.
    fn handle_approval_button(
        &self,
        interaction: &types::Interaction,
        custom_id: &str,
        api: &DiscordApi,
    ) -> Result<(), SysError> {
        let parts: Vec<&str> = custom_id.splitn(3, ':').collect();
        if parts.len() < 3 {
            return Err(SysError::ApiError("Malformed approval custom_id".into()));
        }
        let request_id = parts[1];
        let option = parts[2];

        let decision = match option {
            "allow_once" => "allow_once",
            "allow_session" => "allow_session",
            "deny" => "deny",
            _ => "deny",
        };

        // Publish approval decision on IPC.
        ipc::publish_json(
            "approval.response",
            &serde_json::json!({
                "request_id": request_id,
                "decision": decision,
            }),
        )?;

        // ACK the button interaction (update message to disable
        // buttons).
        api.interaction_respond(
            &interaction.id,
            &interaction.token,
            &InteractionResponse {
                response_type: callback_type::UPDATE_MESSAGE,
                data: Some(InteractionCallbackData {
                    content: Some(format!("Approval: **{decision}**")),
                    components: Some(vec![]),
                    ..Default::default()
                }),
            },
        )?;

        Ok(())
    }

    /// Handle an elicitation button press.
    fn handle_elicitation_button(
        &self,
        interaction: &types::Interaction,
        custom_id: &str,
        api: &DiscordApi,
    ) -> Result<(), SysError> {
        let parts: Vec<&str> = custom_id.splitn(3, ':').collect();
        if parts.len() < 3 {
            return Err(SysError::ApiError("Malformed elicitation custom_id".into()));
        }
        let request_id = parts[1];
        let option = parts[2];

        // Publish elicitation response on IPC.
        ipc::publish_json(
            "elicitation.response",
            &serde_json::json!({
                "request_id": request_id,
                "response": option,
            }),
        )?;

        // ACK the button interaction.
        api.interaction_respond(
            &interaction.id,
            &interaction.token,
            &InteractionResponse {
                response_type: callback_type::UPDATE_MESSAGE,
                data: Some(InteractionCallbackData {
                    content: Some(format!("Selected: **{option}**")),
                    components: Some(vec![]),
                    ..Default::default()
                }),
            },
        )?;

        Ok(())
    }

    /// Handle a `MESSAGE_CREATE` event from the Gateway proxy.
    ///
    /// Unlike interactions, messages do not have deferred responses.
    /// Instead, we send a regular "Thinking..." message and edit it
    /// as the agent streams output.
    fn handle_message_create(
        &self,
        event: &serde_json::Value,
        init: &InitState,
    ) -> Result<(), SysError> {
        let payload = event
            .get("payload")
            .ok_or_else(|| SysError::ApiError("Missing message payload".into()))?;

        let user_id = payload["author"]["id"].as_str().unwrap_or_default();
        let channel_id = payload["channel_id"].as_str().unwrap_or_default();
        let content = payload["content"].as_str().unwrap_or_default();
        let guild_id = payload.get("guild_id").and_then(|g| g.as_str());

        // Authorization check (user and guild allowlists).
        if !self.is_user_authorized(user_id, guild_id)? {
            return Ok(());
        }

        // Ignore empty messages (attachments-only, embeds-only).
        if content.is_empty() {
            return Ok(());
        }

        // Determine session scope.
        let scope_str = sys::get_config_string("DISCORD_SESSION_SCOPE").unwrap_or_default();
        let scope = SessionScope::from_config(&scope_str);
        let scope_id = match scope {
            SessionScope::Channel => channel_id,
            SessionScope::User => user_id,
        };

        // Check for existing session / turn state.
        let existing_session = get_session(scope, scope_id)?;

        if let Some(ref s) = existing_session
            && s.turn_in_progress
        {
            return Ok(());
        }

        // Send acknowledgment as a regular channel message.
        let api = DiscordApi::from_config()?;
        let ack = api.send_message(channel_id, "\u{23f3} Thinking...")?;
        let ack_id = ack["id"].as_str().unwrap_or_default().to_string();

        // Route to agent runtime via Uplink.
        let _ = uplink::send_bytes(
            init.connector_id.as_bytes(),
            user_id.as_bytes(),
            content.as_bytes(),
        )?;

        // Update (or create) session state.
        let session_id = existing_session
            .as_ref()
            .map(|s| s.session_id.clone())
            .unwrap_or_else(|| format!("discord-{scope_id}"));

        let state = SessionState {
            session_id,
            connector_id: init.connector_id.clone(),
            last_message_id: Some(ack_id),
            turn_in_progress: true,
            interaction_token: None, // No interaction token for messages.
        };
        set_session(scope, scope_id, &state)?;

        // Store the active turn metadata.
        set_active_turn(&ActiveTurn {
            scope: scope_str,
            scope_id: scope_id.to_string(),
            buffer: String::new(),
            channel_id: channel_id.to_string(),
        })?;

        Ok(())
    }

    /// Check whether a user/guild is authorized (without interaction
    /// response). Returns `true` if authorized.
    fn is_user_authorized(&self, user_id: &str, guild_id: Option<&str>) -> Result<bool, SysError> {
        let allowed_users = sys::get_config_string("DISCORD_ALLOWED_USERS").unwrap_or_default();
        if !allowed_users.is_empty() {
            let allowed: Vec<&str> = allowed_users.split(',').map(str::trim).collect();
            if !allowed.contains(&user_id) {
                return Ok(false);
            }
        }

        let allowed_guilds = sys::get_config_string("DISCORD_ALLOWED_GUILDS").unwrap_or_default();
        if !allowed_guilds.is_empty() {
            let gid = guild_id.unwrap_or("");
            let allowed: Vec<&str> = allowed_guilds.split(',').map(str::trim).collect();
            if !allowed.contains(&gid) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Handle a text chunk from the agent output stream.
    fn handle_text_chunk(&self, event: &serde_json::Value) -> Result<(), SysError> {
        let chunk = event.get("content").and_then(|v| v.as_str()).unwrap_or("");

        if chunk.is_empty() {
            return Ok(());
        }

        // Load the active turn to get scope info and buffer.
        let mut turn = match get_active_turn()? {
            Some(t) => t,
            None => return Ok(()),
        };

        // Accumulate text.
        turn.buffer.push_str(chunk);
        set_active_turn(&turn)?;

        // Find the session to determine response mode.
        let scope = SessionScope::from_config(&turn.scope);

        if let Some(session) = get_session(scope, &turn.scope_id)? {
            let api = DiscordApi::from_config()?;
            let sanitized = sanitize_for_discord(&turn.buffer);
            let chunks = chunk_discord(&sanitized, 0);
            let display = chunks.first().cloned().unwrap_or_default();

            if let Some(ref token) = session.interaction_token {
                // Interaction mode: edit deferred response via webhook.
                api.interaction_edit_original(
                    token,
                    &WebhookMessage {
                        content: Some(display),
                        ..Default::default()
                    },
                )?;
            } else if let Some(ref msg_id) = session.last_message_id {
                // Message mode: edit the "Thinking..." message.
                let channel_id = &turn.channel_id;
                api.edit_message(channel_id, msg_id, &display)?;
            }
        }

        Ok(())
    }

    /// Handle turn completion.
    fn handle_turn_complete(&self, _event: &serde_json::Value) -> Result<(), SysError> {
        // Load the active turn to get scope info and buffer.
        let turn = match get_active_turn()? {
            Some(t) => t,
            None => return Ok(()),
        };

        let scope = SessionScope::from_config(&turn.scope);

        if let Some(mut session) = get_session(scope, &turn.scope_id)? {
            if !turn.buffer.is_empty() {
                let api = DiscordApi::from_config()?;
                let sanitized = sanitize_for_discord(&turn.buffer);
                let chunks = chunk_discord(&sanitized, 0);

                if let Some(ref token) = session.interaction_token {
                    // Interaction mode: edit deferred response + followups.
                    if let Some(first) = chunks.first() {
                        api.interaction_edit_original(
                            token,
                            &WebhookMessage {
                                content: Some(first.clone()),
                                ..Default::default()
                            },
                        )?;
                    }
                    for followup_chunk in chunks.iter().skip(1) {
                        api.interaction_followup(
                            token,
                            &WebhookMessage {
                                content: Some(followup_chunk.clone()),
                                ..Default::default()
                            },
                        )?;
                    }
                } else if let Some(ref msg_id) = session.last_message_id {
                    // Message mode: edit the ack message + send extras.
                    let channel_id = &turn.channel_id;
                    if let Some(first) = chunks.first() {
                        api.edit_message(channel_id, msg_id, first)?;
                    }
                    for followup_chunk in chunks.iter().skip(1) {
                        api.send_message(channel_id, followup_chunk)?;
                    }
                }
            }

            // Mark turn as complete.
            session.turn_in_progress = false;
            session.interaction_token = None;
            session.last_message_id = None;
            set_session(scope, &turn.scope_id, &session)?;
        }

        // Clear the active turn.
        clear_active_turn()?;

        Ok(())
    }

    /// Handle an error event from the runtime.
    fn handle_error_event(
        &self,
        event: &serde_json::Value,
        _init: &InitState,
    ) -> Result<(), SysError> {
        let error_msg = event
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");

        // Load the active turn to get scope info.
        let turn = match get_active_turn()? {
            Some(t) => t,
            None => {
                sys::log("error", format!("Error without active turn: {error_msg}"))?;
                return Ok(());
            },
        };

        let api = DiscordApi::from_config()?;
        let scope = SessionScope::from_config(&turn.scope);
        let error_text = format!("**Error:** {error_msg}");

        // Edit the response with the error message.
        if let Some(session) = get_session(scope, &turn.scope_id)? {
            if let Some(ref token) = session.interaction_token {
                // Interaction mode: edit deferred response.
                if let Err(e) = api.interaction_edit_original(
                    token,
                    &WebhookMessage {
                        content: Some(error_text.clone()),
                        ..Default::default()
                    },
                ) {
                    sys::log("error", format!("Failed to edit error response: {e}"))?;
                }
            } else if let Some(ref msg_id) = session.last_message_id {
                // Message mode: edit the ack message.
                let channel_id = &turn.channel_id;
                if let Err(e) = api.edit_message(channel_id, msg_id, &error_text) {
                    sys::log("error", format!("Failed to edit error message: {e}"))?;
                }
            } else {
                // Fallback: send to channel from event payload.
                let channel_id = event
                    .get("channel_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !channel_id.is_empty()
                    && let Err(e) = api.send_message(channel_id, &error_text)
                {
                    sys::log("error", format!("Failed to send error message: {e}"))?;
                }
            }

            // Clear turn state on the session.
            let mut session = session;
            session.turn_in_progress = false;
            session.interaction_token = None;
            session.last_message_id = None;
            set_session(scope, &turn.scope_id, &session)?;
        }

        // Clear the active turn.
        clear_active_turn()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ToolOutput serialization ---

    #[test]
    fn tool_output_success_serializes() {
        let output = ToolOutput {
            content: "result data".to_string(),
            is_error: false,
        };
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["content"], "result data");
        assert_eq!(json["is_error"], false);
    }

    #[test]
    fn tool_output_error_serializes() {
        let output = ToolOutput {
            content: "something failed".to_string(),
            is_error: true,
        };
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["content"], "something failed");
        assert_eq!(json["is_error"], true);
    }

    // --- SendArgs deserialization ---

    #[test]
    fn send_args_deserializes() {
        let json = serde_json::json!({
            "channel_id": "ch-123",
            "content": "Hello, Discord!"
        });
        let args: SendArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.channel_id, "ch-123");
        assert_eq!(args.content, "Hello, Discord!");
    }

    #[test]
    fn send_args_default() {
        let args = SendArgs::default();
        assert!(args.channel_id.is_empty());
        assert!(args.content.is_empty());
    }

    // --- EditArgs deserialization ---

    #[test]
    fn edit_args_deserializes() {
        let json = serde_json::json!({
            "channel_id": "ch-456",
            "message_id": "msg-789",
            "content": "Updated content"
        });
        let args: EditArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.channel_id, "ch-456");
        assert_eq!(args.message_id, "msg-789");
        assert_eq!(args.content, "Updated content");
    }

    #[test]
    fn edit_args_default() {
        let args = EditArgs::default();
        assert!(args.channel_id.is_empty());
        assert!(args.message_id.is_empty());
        assert!(args.content.is_empty());
    }

    // --- EmptyArgs ---

    #[test]
    fn empty_args_from_empty_json() {
        let json = serde_json::json!({});
        let _args: EmptyArgs = serde_json::from_value(json).unwrap();
    }

    #[test]
    fn empty_args_default() {
        let _args = EmptyArgs::default();
    }
}
