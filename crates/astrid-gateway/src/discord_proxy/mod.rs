//! Discord Gateway proxy — host-side `WebSocket` relay.
//!
//! Maintains a persistent outbound `WebSocket` connection to Discord's
//! Gateway and relays events to the Discord capsule via the IPC
//! [`EventBus`]. The capsule receives events through its existing
//! `ipc::poll_bytes()` mechanism on cron ticks.
//!
//! # Lifecycle
//!
//! The proxy is started when the `astrid-discord` capsule is loaded and
//! stopped when the capsule is unloaded or the daemon shuts down. It
//! handles reconnection (with resume) and zombie detection internally.
//!
//! [`EventBus`]: astrid_events::EventBus

mod backoff;
mod connection;
pub(crate) mod error;
mod heartbeat;
pub(crate) mod protocol;

use std::sync::Arc;
use std::time::Duration;

use astrid_events::{AstridEvent, EventBus, EventMetadata, IpcMessage, IpcPayload};
use futures::StreamExt;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

use self::backoff::Backoff;
use self::connection::GatewayConnection;
use self::error::DiscordProxyError;
use self::heartbeat::HeartbeatState;
use self::protocol::{GatewayPayload, HelloPayload, ReadyPayload, opcode};

/// Maximum event payload size relayed to the capsule (5 MB).
const MAX_EVENT_PAYLOAD_BYTES: usize = 5 * 1024 * 1024;

/// Timeout for receiving Hello after `WebSocket` connect.
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);

/// Type alias for the split `WebSocket` reader.
type WsReader = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

// ── Configuration ────────────────────────────────────────────

/// Configuration for the Discord Gateway proxy.
pub struct DiscordProxyConfig {
    /// Bot token (from capsule env `DISCORD_BOT_TOKEN`).
    pub bot_token: String,
    /// Application ID (from capsule env `DISCORD_APPLICATION_ID`).
    pub application_id: String,
    /// Gateway intents bitmask.
    pub intents: u32,
    /// Capsule ID for IPC topic prefixing.
    pub capsule_id: String,
    /// Maximum reconnection attempts before giving up.
    pub max_reconnect_attempts: u32,
    /// Base delay for exponential backoff (milliseconds).
    pub backoff_base_ms: u64,
    /// Maximum backoff delay (milliseconds).
    pub backoff_max_ms: u64,
}

impl Default for DiscordProxyConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            application_id: String::new(),
            intents: protocol::DEFAULT_INTENTS,
            capsule_id: "astrid-discord".to_string(),
            max_reconnect_attempts: u32::MAX,
            backoff_base_ms: 1000,
            backoff_max_ms: 60_000,
        }
    }
}

// ── Gateway State ────────────────────────────────────────────

/// Persistent state for resume across reconnections.
struct GatewayState {
    /// Discord session ID from `READY` event.
    session_id: Option<String>,
    /// Last received sequence number.
    sequence: Option<u64>,
    /// URL to use for resume (from `READY` event).
    resume_gateway_url: Option<String>,
    /// The bot's own user ID (for self-message filtering).
    bot_user_id: Option<String>,
}

impl GatewayState {
    fn new() -> Self {
        Self {
            session_id: None,
            sequence: None,
            resume_gateway_url: None,
            bot_user_id: None,
        }
    }

    /// Clear session state for a full reconnect.
    fn clear_session(&mut self) {
        self.session_id = None;
        self.resume_gateway_url = None;
    }

    /// Returns `true` if a resume is possible.
    fn can_resume(&self) -> bool {
        self.session_id.is_some() && self.resume_gateway_url.is_some()
    }
}

// ── Proxy ────────────────────────────────────────────────────

/// Discord Gateway proxy.
///
/// Maintains an outbound `WebSocket` to Discord's Gateway and relays
/// events to the capsule via the IPC event bus.
pub struct DiscordGatewayProxy {
    config: DiscordProxyConfig,
    event_bus: EventBus,
    state: GatewayState,
    shutdown_rx: broadcast::Receiver<()>,
    shutdown_tx: broadcast::Sender<()>,
    /// Stable UUID identifying this proxy instance on the event bus.
    proxy_uuid: Uuid,
    /// HTTP client for fetching the gateway URL.
    http: reqwest::Client,
}

impl DiscordGatewayProxy {
    /// Create a new proxy. Does not connect yet.
    #[must_use]
    pub fn new(
        config: DiscordProxyConfig,
        event_bus: EventBus,
        shutdown_rx: broadcast::Receiver<()>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        Self {
            config,
            event_bus,
            state: GatewayState::new(),
            shutdown_rx,
            shutdown_tx,
            proxy_uuid: Uuid::new_v4(),
            http: reqwest::Client::new(),
        }
    }

    /// Run the proxy.
    ///
    /// Connects, identifies, and enters the event loop. Handles
    /// reconnection internally. Returns only on shutdown or
    /// unrecoverable error.
    ///
    /// # Errors
    ///
    /// Returns `DiscordProxyError` on fatal errors (authentication
    /// failure, invalid intents, max reconnect attempts exceeded).
    pub async fn run(&mut self) -> Result<(), DiscordProxyError> {
        let mut backoff = Backoff::new(self.config.backoff_base_ms, self.config.backoff_max_ms);
        let mut attempt: u32 = 0;

        loop {
            if self.is_shutdown() {
                return Ok(());
            }

            let result = match self.connect_and_run().await {
                Ok(action) => {
                    self.handle_loop_action(action, &mut backoff, &mut attempt)
                        .await
                },
                Err(e) => self.handle_loop_error(e, &mut backoff, &mut attempt).await,
            };

            match result {
                Ok(()) => {},
                Err(DiscordProxyError::Shutdown) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    /// Process a loop action from `connect_and_run`.
    async fn handle_loop_action(
        &mut self,
        action: LoopAction,
        backoff: &mut Backoff,
        attempt: &mut u32,
    ) -> Result<(), DiscordProxyError> {
        match action {
            LoopAction::Shutdown => Err(DiscordProxyError::Shutdown),
            LoopAction::Resume => {
                let delay = Duration::from_millis(fastrand::u64(1000..=5000));
                info!(delay_ms = delay.as_millis(), "Attempting resume");
                self.publish_status("resuming", Some("will_resume"));
                self.sleep_or_shutdown(delay).await
            },
            LoopAction::Reconnect => {
                self.state.clear_session();
                let delay = backoff.next_delay();
                *attempt = attempt.saturating_add(1);

                if *attempt > self.config.max_reconnect_attempts {
                    error!(
                        "Max reconnect attempts ({}) exceeded",
                        self.config.max_reconnect_attempts
                    );
                    return Err(DiscordProxyError::Protocol(
                        "Max reconnect attempts exceeded".into(),
                    ));
                }

                info!(
                    delay_ms = delay.as_millis(),
                    attempt = *attempt,
                    "Reconnecting after backoff"
                );
                self.publish_status("disconnected", Some("reconnecting"));
                self.sleep_or_shutdown(delay).await
            },
            LoopAction::Connected => {
                backoff.reset();
                *attempt = 0;
                Ok(())
            },
        }
    }

    /// Process an error from `connect_and_run`.
    async fn handle_loop_error(
        &mut self,
        err: DiscordProxyError,
        backoff: &mut Backoff,
        attempt: &mut u32,
    ) -> Result<(), DiscordProxyError> {
        match &err {
            DiscordProxyError::AuthenticationFailed
            | DiscordProxyError::InvalidIntents(_)
            | DiscordProxyError::UnrecoverableClose(_) => {
                error!(error = %err, "Fatal Gateway error");
                self.publish_status_error(&err.to_string());
                Err(err)
            },
            DiscordProxyError::Shutdown => Err(err),
            _ => {
                warn!(error = %err, "Gateway connection error");
                self.state.clear_session();
                let delay = backoff.next_delay();
                *attempt = attempt.saturating_add(1);
                info!(
                    delay_ms = delay.as_millis(),
                    attempt = *attempt,
                    "Reconnecting after error"
                );
                self.sleep_or_shutdown(delay).await
            },
        }
    }

    /// Single connection attempt: connect, handshake, run event loop.
    async fn connect_and_run(&mut self) -> Result<LoopAction, DiscordProxyError> {
        let gateway_url = self.resolve_gateway_url().await?;
        let ws_url = format!("{gateway_url}?v=10&encoding=json");
        info!(url = %ws_url, "Connecting to Discord Gateway");
        self.publish_status("connecting", None);

        let conn = GatewayConnection::connect(&ws_url).await?;
        let (ws_writer, mut ws_reader) = conn.into_parts();

        let hello = self.wait_for_hello(&mut ws_reader).await?;
        let interval_ms = hello.heartbeat_interval;

        let sequence = Arc::new(Mutex::new(self.state.sequence));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));
        let (outbound_tx, outbound_rx) = mpsc::channel::<GatewayPayload>(64);
        let (zombie_tx, zombie_rx) = oneshot::channel();

        let heartbeat_handle = self.spawn_heartbeat(
            interval_ms,
            Arc::clone(&sequence),
            Arc::clone(&hb_state),
            outbound_tx.clone(),
            zombie_tx,
        );

        let identify_payload = self.build_auth_payload();
        outbound_tx
            .send(identify_payload)
            .await
            .map_err(|_| DiscordProxyError::Protocol("Writer channel closed".into()))?;

        let is_resuming = self.state.can_resume();
        let mut writer_handle = Self::spawn_writer(ws_writer, outbound_rx);

        let action = self
            .event_loop(
                &mut ws_reader,
                &outbound_tx,
                &sequence,
                &hb_state,
                zombie_rx,
                is_resuming,
            )
            .await;

        heartbeat_handle.abort();

        // Drop the outbound channel so the writer sees EOF, then give
        // it 2 seconds to flush remaining payloads before aborting.
        drop(outbound_tx);
        tokio::select! {
            _ = &mut writer_handle => {},
            () = tokio::time::sleep(Duration::from_secs(2)) => {
                writer_handle.abort();
            },
        }

        action
    }

    /// Resolve the gateway URL (resume URL or fresh fetch).
    async fn resolve_gateway_url(&mut self) -> Result<String, DiscordProxyError> {
        if !self.state.can_resume() {
            return self.fetch_gateway_url().await;
        }

        let url = self.state.resume_gateway_url.clone().unwrap_or_default();

        if protocol::is_valid_resume_url(&url) {
            Ok(url)
        } else {
            warn!(url = %url, "Invalid resume URL, fetching fresh");
            self.state.clear_session();
            self.fetch_gateway_url().await
        }
    }

    /// Build the Identify or Resume payload.
    fn build_auth_payload(&self) -> GatewayPayload {
        if self.state.can_resume() {
            let session_id = self.state.session_id.as_deref().unwrap_or("");
            let seq = self.state.sequence.unwrap_or(0);
            protocol::build_resume(&self.config.bot_token, session_id, seq)
        } else {
            protocol::build_identify(&self.config.bot_token, self.config.intents)
        }
    }

    /// Spawn the heartbeat background task.
    fn spawn_heartbeat(
        &self,
        interval_ms: u64,
        sequence: Arc<Mutex<Option<u64>>>,
        hb_state: Arc<Mutex<HeartbeatState>>,
        outbound_tx: mpsc::Sender<GatewayPayload>,
        zombie_tx: oneshot::Sender<()>,
    ) -> tokio::task::JoinHandle<()> {
        let shutdown_rx = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            heartbeat::run_heartbeat(
                interval_ms,
                sequence,
                hb_state,
                outbound_tx,
                zombie_tx,
                shutdown_rx,
            )
            .await;
        })
    }

    /// Spawn the `WebSocket` writer task.
    fn spawn_writer(
        ws_writer: futures::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        mut outbound_rx: mpsc::Receiver<GatewayPayload>,
    ) -> tokio::task::JoinHandle<()> {
        use futures::SinkExt;

        tokio::spawn(async move {
            let mut ws_writer = ws_writer;
            while let Some(payload) = outbound_rx.recv().await {
                let json = match serde_json::to_string(&payload) {
                    Ok(j) => j,
                    Err(e) => {
                        error!(error = %e, "Failed to serialize Gateway payload");
                        continue;
                    }
                };
                if let Err(e) = ws_writer.send(Message::Text(json.into())).await {
                    debug!(error = %e, "Writer task: send failed");
                    break;
                }
            }
        })
    }

    /// Main event loop: reads events, relays to capsule, handles
    /// protocol messages.
    #[allow(clippy::too_many_arguments)]
    async fn event_loop(
        &mut self,
        ws_reader: &mut WsReader,
        outbound_tx: &mpsc::Sender<GatewayPayload>,
        sequence: &Arc<Mutex<Option<u64>>>,
        hb_state: &Arc<Mutex<HeartbeatState>>,
        mut zombie_rx: oneshot::Receiver<()>,
        _is_resuming: bool,
    ) -> Result<LoopAction, DiscordProxyError> {
        loop {
            tokio::select! {
                biased;

                _ = self.shutdown_rx.recv() => {
                    info!("Gateway proxy received shutdown signal");
                    return Ok(LoopAction::Shutdown);
                }

                _ = &mut zombie_rx => {
                    warn!("Zombie connection detected — reconnecting");
                    return Ok(LoopAction::Resume);
                }

                msg = ws_reader.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            let payload: GatewayPayload =
                                match serde_json::from_str(&text) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        warn!(
                                            error = %e,
                                            "Failed to parse \
                                             Gateway payload"
                                        );
                                        continue;
                                    }
                                };

                            if let Some(action) = self
                                .handle_payload(
                                    payload,
                                    outbound_tx,
                                    sequence,
                                    hb_state,
                                )
                                .await?
                            {
                                return Ok(action);
                            }
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let code = frame
                                .as_ref()
                                .map_or(1000, |f| f.code.into());
                            return self.handle_close_code(code);
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            warn!(error = %e, "WebSocket read error");
                            return Ok(self.resume_or_reconnect());
                        }
                        None => {
                            warn!("WebSocket stream ended");
                            return Ok(self.resume_or_reconnect());
                        }
                    }
                }
            }
        }
    }

    /// Choose resume or reconnect based on current state.
    fn resume_or_reconnect(&self) -> LoopAction {
        if self.state.can_resume() {
            LoopAction::Resume
        } else {
            LoopAction::Reconnect
        }
    }

    /// Handle a single Gateway payload.
    ///
    /// Returns `Some(LoopAction)` if the event loop should break.
    async fn handle_payload(
        &mut self,
        payload: GatewayPayload,
        outbound_tx: &mpsc::Sender<GatewayPayload>,
        sequence: &Arc<Mutex<Option<u64>>>,
        hb_state: &Arc<Mutex<HeartbeatState>>,
    ) -> Result<Option<LoopAction>, DiscordProxyError> {
        match payload.op {
            opcode::DISPATCH => self.handle_dispatch(payload, sequence).await,
            opcode::HEARTBEAT => {
                let seq = *sequence.lock().await;
                let hb = protocol::build_heartbeat(seq);
                let _ = outbound_tx.send(hb).await;
                Ok(None)
            },
            opcode::HEARTBEAT_ACK => {
                hb_state.lock().await.ack_received();
                Ok(None)
            },
            opcode::RECONNECT => {
                info!("Server requested reconnect (op=7)");
                Ok(Some(LoopAction::Resume))
            },
            opcode::INVALID_SESSION => Ok(Some(self.handle_invalid_session(&payload))),
            opcode::HELLO => {
                warn!("Unexpected Hello (op=10) mid-session");
                Ok(None)
            },
            _ => {
                debug!(op = payload.op, "Unknown Gateway opcode");
                Ok(None)
            },
        }
    }

    /// Handle a dispatch event (op=0).
    async fn handle_dispatch(
        &mut self,
        payload: GatewayPayload,
        sequence: &Arc<Mutex<Option<u64>>>,
    ) -> Result<Option<LoopAction>, DiscordProxyError> {
        if let Some(seq) = payload.s {
            *sequence.lock().await = Some(seq);
            self.state.sequence = Some(seq);
        }

        let event_name = payload.t.as_deref().unwrap_or("");

        match event_name {
            "READY" => {
                self.handle_ready(&payload)?;
                Ok(Some(LoopAction::Connected))
            },
            "RESUMED" => {
                info!("Gateway session resumed");
                self.publish_status("connected", Some("resumed"));
                Ok(Some(LoopAction::Connected))
            },
            "MESSAGE_CREATE" => {
                self.relay_message_create(&payload);
                Ok(None)
            },
            "INTERACTION_CREATE" => {
                self.relay_interaction_create(&payload);
                Ok(None)
            },
            _ => {
                trace!(event = event_name, "Ignoring Gateway dispatch");
                Ok(None)
            },
        }
    }

    /// Handle an invalid session event (op=9).
    fn handle_invalid_session(&mut self, payload: &GatewayPayload) -> LoopAction {
        let resumable = payload
            .d
            .as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if resumable {
            info!("Invalid session (resumable) — will resume");
            LoopAction::Resume
        } else {
            info!("Invalid session (not resumable) — full reconnect");
            self.state.clear_session();
            LoopAction::Reconnect
        }
    }

    /// Handle the `READY` dispatch event.
    fn handle_ready(&mut self, payload: &GatewayPayload) -> Result<(), DiscordProxyError> {
        let data = payload
            .d
            .as_ref()
            .ok_or_else(|| DiscordProxyError::Protocol("READY event missing data".into()))?;

        let ready: ReadyPayload = serde_json::from_value(data.clone())?;

        info!(
            session_id = %ready.session_id,
            bot_user_id = %ready.user.id,
            "Gateway session established (READY)"
        );

        self.state.session_id = Some(ready.session_id.clone());
        self.state.bot_user_id = Some(ready.user.id);

        if protocol::is_valid_resume_url(&ready.resume_gateway_url) {
            self.state.resume_gateway_url = Some(ready.resume_gateway_url);
        } else {
            warn!(
                url = %ready.resume_gateway_url,
                "READY contained invalid resume URL — ignoring"
            );
        }

        self.publish_status("connected", Some("ready"));
        Ok(())
    }

    /// Relay a `MESSAGE_CREATE` dispatch to the capsule.
    fn relay_message_create(&self, payload: &GatewayPayload) {
        let Some(data) = &payload.d else { return };

        let author_id = data
            .get("author")
            .and_then(|a| a.get("id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Filter out the bot's own messages.
        if let Some(ref bot_id) = self.state.bot_user_id
            && author_id == bot_id
        {
            trace!("Filtering self-message");
            return;
        }

        // Filter out bot messages (`author.bot` == true).
        let is_bot = data
            .get("author")
            .and_then(|a| a.get("bot"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if is_bot {
            trace!("Filtering bot message");
            return;
        }

        let event = serde_json::json!({
            "type": "message",
            "source": "gateway",
            "payload": {
                "id": data.get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                "channel_id": data.get("channel_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                "guild_id": data.get("guild_id")
                    .and_then(serde_json::Value::as_str),
                "author": {
                    "id": author_id,
                    "username": data.get("author")
                        .and_then(|a| a.get("username"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(""),
                },
                "content": data.get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                "timestamp": data.get("timestamp")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            },
        });

        debug!(
            event = "MESSAGE_CREATE",
            "Relaying Gateway event to capsule"
        );
        self.relay_event(event);
    }

    /// Relay an `INTERACTION_CREATE` dispatch to the capsule.
    fn relay_interaction_create(&self, payload: &GatewayPayload) {
        let Some(data) = &payload.d else { return };

        let event = serde_json::json!({
            "type": "interaction",
            "source": "gateway",
            "payload": data,
        });

        debug!(
            event = "INTERACTION_CREATE",
            "Relaying Gateway event to capsule"
        );
        self.relay_event(event);
    }

    /// Publish an event to the capsule's IPC topic.
    fn relay_event(&self, event_data: serde_json::Value) {
        let size = event_data.to_string().len();
        if size > MAX_EVENT_PAYLOAD_BYTES {
            warn!(size, "Dropping oversized Gateway event (>5MB)");
            return;
        }

        let topic = format!("{}.agent.events", self.config.capsule_id);
        let message = IpcMessage::new(
            topic,
            IpcPayload::Custom { data: event_data },
            self.proxy_uuid,
        );
        let event = AstridEvent::Ipc {
            metadata: EventMetadata::new("discord-gateway"),
            message,
        };
        self.event_bus.publish(event);
    }

    /// Publish a status update on the IPC status topic.
    fn publish_status(&self, state: &str, detail: Option<&str>) {
        let mut data = serde_json::json!({ "state": state });
        if let Some(d) = detail {
            data["detail"] = serde_json::Value::from(d);
        }
        if let Some(ref sid) = self.state.session_id {
            data["session_id"] = serde_json::Value::from(sid.as_str());
        }

        let topic = format!("{}.gateway.status", self.config.capsule_id);
        let message = IpcMessage::new(topic, IpcPayload::Custom { data }, self.proxy_uuid);
        let event = AstridEvent::Ipc {
            metadata: EventMetadata::new("discord-gateway"),
            message,
        };
        self.event_bus.publish(event);
    }

    /// Publish a fatal error status.
    fn publish_status_error(&self, message: &str) {
        let data = serde_json::json!({
            "state": "error",
            "message": message,
            "fatal": true,
        });
        let topic = format!("{}.gateway.status", self.config.capsule_id);
        let ipc_message = IpcMessage::new(topic, IpcPayload::Custom { data }, self.proxy_uuid);
        let event = AstridEvent::Ipc {
            metadata: EventMetadata::new("discord-gateway"),
            message: ipc_message,
        };
        self.event_bus.publish(event);
    }

    /// Fetch the gateway URL from Discord's REST API.
    async fn fetch_gateway_url(&self) -> Result<String, DiscordProxyError> {
        let resp = self
            .http
            .get("https://discord.com/api/v10/gateway/bot")
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .send()
            .await?;

        if resp.status() == 401 {
            return Err(DiscordProxyError::AuthenticationFailed);
        }

        let body: protocol::GatewayBotResponse = resp.json().await?;
        Ok(body.url)
    }

    /// Wait for the Hello payload after connecting.
    async fn wait_for_hello(
        &self,
        ws_reader: &mut WsReader,
    ) -> Result<HelloPayload, DiscordProxyError> {
        let hello_fut = async {
            loop {
                match ws_reader.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let payload: GatewayPayload = serde_json::from_str(&text)?;
                        if payload.op == opcode::HELLO {
                            let data = payload.d.ok_or_else(|| {
                                DiscordProxyError::Protocol("Hello missing data".into())
                            })?;
                            let hello: HelloPayload = serde_json::from_value(data)?;
                            return Ok(hello);
                        }
                    },
                    Some(Ok(_)) => {},
                    Some(Err(e)) => {
                        return Err(e.into());
                    },
                    None => {
                        return Err(DiscordProxyError::Protocol(
                            "Connection closed before Hello".into(),
                        ));
                    },
                }
            }
        };

        tokio::time::timeout(HELLO_TIMEOUT, hello_fut)
            .await
            .map_err(|_| DiscordProxyError::HelloTimeout)?
    }

    /// Classify a close code into the appropriate loop action.
    fn handle_close_code(&mut self, code: u16) -> Result<LoopAction, DiscordProxyError> {
        use protocol::close_code;

        match code {
            close_code::AUTHENTICATION_FAILED => Err(DiscordProxyError::AuthenticationFailed),
            close_code::INVALID_SHARD => Err(DiscordProxyError::UnrecoverableClose(code)),
            close_code::INVALID_INTENTS | close_code::DISALLOWED_INTENTS => {
                Err(DiscordProxyError::InvalidIntents(code))
            },
            1000 | 1001 => {
                info!(code, "Normal close — full reconnect");
                self.state.clear_session();
                Ok(LoopAction::Reconnect)
            },
            _ => {
                warn!(code, "Close code received — attempting resume");
                Ok(self.resume_or_reconnect())
            },
        }
    }

    /// Sleep for a duration, or return early on shutdown.
    async fn sleep_or_shutdown(&mut self, duration: Duration) -> Result<(), DiscordProxyError> {
        tokio::select! {
            biased;
            _ = self.shutdown_rx.recv() => {
                Err(DiscordProxyError::Shutdown)
            }
            () = tokio::time::sleep(duration) => {
                Ok(())
            }
        }
    }

    /// Check if a shutdown has been signalled.
    fn is_shutdown(&self) -> bool {
        self.shutdown_tx.receiver_count() == 0 && !self.shutdown_rx.is_empty()
    }
}

/// What the outer reconnection loop should do next.
enum LoopAction {
    /// Graceful shutdown — return `Ok(())`.
    Shutdown,
    /// Attempt resume (keep session state).
    Resume,
    /// Full reconnect (clear session state).
    Reconnect,
    /// Successfully connected — reset backoff.
    Connected,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────

    /// Create a proxy wired to a fresh `EventBus` for testing.
    fn test_proxy(event_bus: &EventBus) -> DiscordGatewayProxy {
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        DiscordGatewayProxy::new(
            DiscordProxyConfig {
                bot_token: "test-token".into(),
                application_id: "app-123".into(),
                capsule_id: "test-capsule".into(),
                ..Default::default()
            },
            event_bus.clone(),
            shutdown_rx,
            shutdown_tx,
        )
    }

    /// Extract the IPC event data from an `AstridEvent::Ipc`.
    fn extract_ipc(event: &AstridEvent) -> Option<(&str, &serde_json::Value)> {
        if let AstridEvent::Ipc { message, .. } = event {
            if let IpcPayload::Custom { data } = &message.payload {
                return Some((&message.topic, data));
            }
        }
        None
    }

    // ── Config Tests ────────────────────────────────────────

    #[test]
    fn default_config_values() {
        let config = DiscordProxyConfig::default();
        assert_eq!(config.intents, protocol::DEFAULT_INTENTS);
        assert_eq!(config.capsule_id, "astrid-discord");
        assert_eq!(config.max_reconnect_attempts, u32::MAX);
        assert_eq!(config.backoff_base_ms, 1000);
        assert_eq!(config.backoff_max_ms, 60_000);
    }

    #[test]
    fn max_event_payload_is_5mb() {
        assert_eq!(MAX_EVENT_PAYLOAD_BYTES, 5 * 1024 * 1024);
    }

    // ── GatewayState Tests ──────────────────────────────────

    #[test]
    fn gateway_state_can_resume() {
        let mut state = GatewayState::new();
        assert!(!state.can_resume());

        state.session_id = Some("sess".to_string());
        assert!(!state.can_resume());

        state.resume_gateway_url = Some("wss://gw.discord.gg".to_string());
        assert!(state.can_resume());
    }

    #[test]
    fn gateway_state_clear_session() {
        let mut state = GatewayState::new();
        state.session_id = Some("sess".to_string());
        state.resume_gateway_url = Some("wss://gw.discord.gg".to_string());
        state.sequence = Some(42);
        state.bot_user_id = Some("bot-id".to_string());

        state.clear_session();

        assert!(state.session_id.is_none());
        assert!(state.resume_gateway_url.is_none());
        assert_eq!(state.sequence, Some(42));
        assert_eq!(state.bot_user_id.as_deref(), Some("bot-id"));
    }

    #[test]
    fn gateway_state_new_is_empty() {
        let state = GatewayState::new();
        assert!(state.session_id.is_none());
        assert!(state.sequence.is_none());
        assert!(state.resume_gateway_url.is_none());
        assert!(state.bot_user_id.is_none());
    }

    // ── relay_message_create Tests ──────────────────────────

    #[test]
    fn relay_message_create_publishes_to_event_bus() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "id": "msg-001",
                "channel_id": "ch-100",
                "guild_id": "guild-1",
                "author": {
                    "id": "user-42",
                    "username": "alice",
                },
                "content": "Hello bot!",
                "timestamp": "2026-01-01T00:00:00Z",
            })),
            s: Some(1),
            t: Some("MESSAGE_CREATE".into()),
        };

        proxy.relay_message_create(&payload);

        let event = receiver.try_recv().unwrap();
        let (topic, data) = extract_ipc(&event).unwrap();
        assert_eq!(topic, "test-capsule.agent.events");
        assert_eq!(data["type"], "message");
        assert_eq!(data["source"], "gateway");
        assert_eq!(data["payload"]["id"], "msg-001");
        assert_eq!(data["payload"]["channel_id"], "ch-100");
        assert_eq!(data["payload"]["author"]["id"], "user-42");
        assert_eq!(data["payload"]["author"]["username"], "alice");
        assert_eq!(data["payload"]["content"], "Hello bot!");
    }

    #[test]
    fn relay_message_create_filters_self_messages() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let mut proxy = test_proxy(&bus);
        proxy.state.bot_user_id = Some("bot-99".into());

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "id": "msg-002",
                "channel_id": "ch-100",
                "author": { "id": "bot-99", "username": "astrid" },
                "content": "I said something",
                "timestamp": "2026-01-01T00:00:00Z",
            })),
            s: Some(2),
            t: Some("MESSAGE_CREATE".into()),
        };

        proxy.relay_message_create(&payload);

        assert!(
            receiver.try_recv().is_none(),
            "Self-message should be filtered"
        );
    }

    #[test]
    fn relay_message_create_filters_bot_messages() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "id": "msg-003",
                "channel_id": "ch-100",
                "author": {
                    "id": "other-bot-55",
                    "username": "webhookbot",
                    "bot": true,
                },
                "content": "Automated message",
                "timestamp": "2026-01-01T00:00:00Z",
            })),
            s: Some(3),
            t: Some("MESSAGE_CREATE".into()),
        };

        proxy.relay_message_create(&payload);

        assert!(
            receiver.try_recv().is_none(),
            "Bot message should be filtered"
        );
    }

    #[test]
    fn relay_message_create_skips_missing_data() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: None,
            s: Some(4),
            t: Some("MESSAGE_CREATE".into()),
        };

        proxy.relay_message_create(&payload);

        assert!(
            receiver.try_recv().is_none(),
            "Missing data should not emit event"
        );
    }

    // ── relay_interaction_create Tests ───────────────────────

    #[test]
    fn relay_interaction_create_publishes_event() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "id": "int-1",
                "type": 2,
                "token": "tok-abc",
                "data": { "name": "chat" },
            })),
            s: Some(5),
            t: Some("INTERACTION_CREATE".into()),
        };

        proxy.relay_interaction_create(&payload);

        let event = receiver.try_recv().unwrap();
        let (topic, data) = extract_ipc(&event).unwrap();
        assert_eq!(topic, "test-capsule.agent.events");
        assert_eq!(data["type"], "interaction");
        assert_eq!(data["source"], "gateway");
        assert_eq!(data["payload"]["id"], "int-1");
    }

    #[test]
    fn relay_interaction_create_skips_missing_data() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: None,
            s: Some(6),
            t: Some("INTERACTION_CREATE".into()),
        };

        proxy.relay_interaction_create(&payload);

        assert!(receiver.try_recv().is_none());
    }

    // ── relay_event (oversized payload) Tests ───────────────

    #[test]
    fn relay_event_drops_oversized_payload() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        // Create a payload larger than 5 MB.
        let big_string = "x".repeat(MAX_EVENT_PAYLOAD_BYTES.saturating_add(1));
        let event_data = serde_json::json!({ "data": big_string });

        proxy.relay_event(event_data);

        assert!(
            receiver.try_recv().is_none(),
            "Oversized payload should be dropped"
        );
    }

    #[test]
    fn relay_event_accepts_payload_under_limit() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        let event_data = serde_json::json!({ "type": "test", "data": "small" });
        proxy.relay_event(event_data);

        assert!(receiver.try_recv().is_some());
    }

    // ── handle_close_code Tests ─────────────────────────────

    #[test]
    fn close_code_auth_failed_is_fatal() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.handle_close_code(4004);
        assert!(matches!(
            result,
            Err(DiscordProxyError::AuthenticationFailed)
        ));
    }

    #[test]
    fn close_code_invalid_shard_is_fatal() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.handle_close_code(4010);
        assert!(matches!(
            result,
            Err(DiscordProxyError::UnrecoverableClose(4010))
        ));
    }

    #[test]
    fn close_code_invalid_intents_is_fatal() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.handle_close_code(4013);
        assert!(matches!(
            result,
            Err(DiscordProxyError::InvalidIntents(4013))
        ));
    }

    #[test]
    fn close_code_disallowed_intents_is_fatal() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.handle_close_code(4014);
        assert!(matches!(
            result,
            Err(DiscordProxyError::InvalidIntents(4014))
        ));
    }

    #[test]
    fn close_code_normal_triggers_reconnect() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.handle_close_code(1000).unwrap();
        assert!(matches!(result, LoopAction::Reconnect));
    }

    #[test]
    fn close_code_going_away_triggers_reconnect() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.handle_close_code(1001).unwrap();
        assert!(matches!(result, LoopAction::Reconnect));
    }

    #[test]
    fn close_code_unknown_attempts_resume_if_session_exists() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        proxy.state.session_id = Some("sess-1".into());
        proxy.state.resume_gateway_url = Some("wss://gw.discord.gg".into());

        let result = proxy.handle_close_code(4001).unwrap();
        assert!(matches!(result, LoopAction::Resume));
    }

    #[test]
    fn close_code_unknown_reconnects_without_session() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.handle_close_code(4001).unwrap();
        assert!(matches!(result, LoopAction::Reconnect));
    }

    // ── handle_invalid_session Tests ────────────────────────

    #[test]
    fn invalid_session_resumable_returns_resume() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 9,
            d: Some(serde_json::Value::Bool(true)),
            s: None,
            t: None,
        };

        let action = proxy.handle_invalid_session(&payload);
        assert!(matches!(action, LoopAction::Resume));
    }

    #[test]
    fn invalid_session_not_resumable_returns_reconnect() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 9,
            d: Some(serde_json::Value::Bool(false)),
            s: None,
            t: None,
        };

        let action = proxy.handle_invalid_session(&payload);
        assert!(matches!(action, LoopAction::Reconnect));
        // Session should be cleared.
        assert!(!proxy.state.can_resume());
    }

    #[test]
    fn invalid_session_missing_data_reconnects() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 9,
            d: None,
            s: None,
            t: None,
        };

        let action = proxy.handle_invalid_session(&payload);
        assert!(matches!(action, LoopAction::Reconnect));
    }

    // ── handle_ready Tests ──────────────────────────────────

    #[test]
    fn handle_ready_stores_session_state() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "session_id": "ready-sess",
                "resume_gateway_url": "wss://gateway.discord.gg",
                "user": { "id": "bot-42" },
                "guilds": [],
                "application": { "id": "app-1" },
            })),
            s: Some(1),
            t: Some("READY".into()),
        };

        proxy.handle_ready(&payload).unwrap();

        assert_eq!(proxy.state.session_id.as_deref(), Some("ready-sess"));
        assert_eq!(proxy.state.bot_user_id.as_deref(), Some("bot-42"));
        assert_eq!(
            proxy.state.resume_gateway_url.as_deref(),
            Some("wss://gateway.discord.gg")
        );
        assert!(proxy.state.can_resume());
    }

    #[test]
    fn handle_ready_rejects_invalid_resume_url() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "session_id": "sess-2",
                "resume_gateway_url": "wss://evil.example.com",
                "user": { "id": "bot-42" },
                "guilds": [],
            })),
            s: Some(1),
            t: Some("READY".into()),
        };

        proxy.handle_ready(&payload).unwrap();

        assert_eq!(proxy.state.session_id.as_deref(), Some("sess-2"));
        assert!(
            proxy.state.resume_gateway_url.is_none(),
            "Invalid resume URL should be rejected"
        );
    }

    #[test]
    fn handle_ready_missing_data_is_error() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let payload = GatewayPayload {
            op: 0,
            d: None,
            s: Some(1),
            t: Some("READY".into()),
        };

        let result = proxy.handle_ready(&payload);
        assert!(result.is_err());
    }

    // ── handle_payload Tests ────────────────────────────────

    #[tokio::test]
    async fn handle_payload_heartbeat_sends_response() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
        let sequence = Arc::new(Mutex::new(Some(42u64)));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));

        let payload = GatewayPayload {
            op: protocol::opcode::HEARTBEAT,
            d: None,
            s: None,
            t: None,
        };

        let result = proxy
            .handle_payload(payload, &outbound_tx, &sequence, &hb_state)
            .await
            .unwrap();

        assert!(result.is_none(), "Heartbeat should not break event loop");

        let sent = outbound_rx.try_recv().unwrap();
        assert_eq!(sent.op, protocol::opcode::HEARTBEAT);
        assert_eq!(sent.d, Some(serde_json::Value::from(42)));
    }

    #[tokio::test]
    async fn handle_payload_heartbeat_ack_updates_state() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let (outbound_tx, _outbound_rx) = mpsc::channel(64);
        let sequence = Arc::new(Mutex::new(None));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));

        hb_state.lock().await.last_ack_received = false;

        let payload = GatewayPayload {
            op: protocol::opcode::HEARTBEAT_ACK,
            d: None,
            s: None,
            t: None,
        };

        proxy
            .handle_payload(payload, &outbound_tx, &sequence, &hb_state)
            .await
            .unwrap();

        assert!(hb_state.lock().await.last_ack_received);
    }

    #[tokio::test]
    async fn handle_payload_reconnect_returns_resume() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let (outbound_tx, _) = mpsc::channel(64);
        let sequence = Arc::new(Mutex::new(None));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));

        let payload = GatewayPayload {
            op: protocol::opcode::RECONNECT,
            d: None,
            s: None,
            t: None,
        };

        let result = proxy
            .handle_payload(payload, &outbound_tx, &sequence, &hb_state)
            .await
            .unwrap();

        assert!(matches!(result, Some(LoopAction::Resume)));
    }

    #[tokio::test]
    async fn handle_payload_unknown_opcode_continues() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let (outbound_tx, _) = mpsc::channel(64);
        let sequence = Arc::new(Mutex::new(None));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));

        let payload = GatewayPayload {
            op: 255,
            d: None,
            s: None,
            t: None,
        };

        let result = proxy
            .handle_payload(payload, &outbound_tx, &sequence, &hb_state)
            .await
            .unwrap();

        assert!(result.is_none());
    }

    // ── handle_dispatch Tests ───────────────────────────────

    #[tokio::test]
    async fn handle_dispatch_ready_returns_connected() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let sequence = Arc::new(Mutex::new(None));

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "session_id": "s1",
                "resume_gateway_url": "wss://gw.discord.gg",
                "user": { "id": "bot-1" },
                "guilds": [],
            })),
            s: Some(1),
            t: Some("READY".into()),
        };

        let result = proxy.handle_dispatch(payload, &sequence).await.unwrap();
        assert!(matches!(result, Some(LoopAction::Connected)));
        assert_eq!(*sequence.lock().await, Some(1));
    }

    #[tokio::test]
    async fn handle_dispatch_resumed_returns_connected() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let sequence = Arc::new(Mutex::new(None));

        let payload = GatewayPayload {
            op: 0,
            d: None,
            s: Some(99),
            t: Some("RESUMED".into()),
        };

        let result = proxy.handle_dispatch(payload, &sequence).await.unwrap();
        assert!(matches!(result, Some(LoopAction::Connected)));
        assert_eq!(*sequence.lock().await, Some(99));
    }

    #[tokio::test]
    async fn handle_dispatch_message_create_relays() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let mut proxy = test_proxy(&bus);
        let sequence = Arc::new(Mutex::new(None));

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({
                "id": "m1",
                "channel_id": "ch-1",
                "author": { "id": "u1", "username": "bob" },
                "content": "hi",
                "timestamp": "2026-01-01T00:00:00Z",
            })),
            s: Some(10),
            t: Some("MESSAGE_CREATE".into()),
        };

        let result = proxy.handle_dispatch(payload, &sequence).await.unwrap();
        assert!(result.is_none(), "MESSAGE_CREATE should not break loop");
        assert_eq!(*sequence.lock().await, Some(10));

        let event = receiver.try_recv().unwrap();
        let (_, data) = extract_ipc(&event).unwrap();
        assert_eq!(data["type"], "message");
    }

    #[tokio::test]
    async fn handle_dispatch_unknown_event_ignored() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let mut proxy = test_proxy(&bus);
        let sequence = Arc::new(Mutex::new(None));

        let payload = GatewayPayload {
            op: 0,
            d: Some(serde_json::json!({})),
            s: Some(20),
            t: Some("GUILD_MEMBER_ADD".into()),
        };

        let result = proxy.handle_dispatch(payload, &sequence).await.unwrap();
        assert!(result.is_none());
        assert_eq!(*sequence.lock().await, Some(20));
        assert!(receiver.try_recv().is_none());
    }

    // ── publish_status Tests ────────────────────────────────

    #[test]
    fn publish_status_emits_on_status_topic() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        proxy.publish_status("connected", Some("ready"));

        let event = receiver.try_recv().unwrap();
        let (topic, data) = extract_ipc(&event).unwrap();
        assert_eq!(topic, "test-capsule.gateway.status");
        assert_eq!(data["state"], "connected");
        assert_eq!(data["detail"], "ready");
    }

    #[test]
    fn publish_status_includes_session_id() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let mut proxy = test_proxy(&bus);
        proxy.state.session_id = Some("sess-abc".into());

        proxy.publish_status("connected", None);

        let event = receiver.try_recv().unwrap();
        let (_, data) = extract_ipc(&event).unwrap();
        assert_eq!(data["session_id"], "sess-abc");
    }

    #[test]
    fn publish_status_error_emits_fatal() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let proxy = test_proxy(&bus);

        proxy.publish_status_error("Token invalid");

        let event = receiver.try_recv().unwrap();
        let (topic, data) = extract_ipc(&event).unwrap();
        assert_eq!(topic, "test-capsule.gateway.status");
        assert_eq!(data["state"], "error");
        assert_eq!(data["message"], "Token invalid");
        assert_eq!(data["fatal"], true);
    }

    // ── build_auth_payload Tests ────────────────────────────

    #[test]
    fn build_auth_payload_identify_when_no_session() {
        let bus = EventBus::new();
        let proxy = test_proxy(&bus);

        let payload = proxy.build_auth_payload();
        assert_eq!(payload.op, protocol::opcode::IDENTIFY);
        let d = payload.d.unwrap();
        assert_eq!(d["token"], "test-token");
    }

    #[test]
    fn build_auth_payload_resume_when_session_exists() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        proxy.state.session_id = Some("s1".into());
        proxy.state.resume_gateway_url = Some("wss://gw.discord.gg".into());
        proxy.state.sequence = Some(55);

        let payload = proxy.build_auth_payload();
        assert_eq!(payload.op, protocol::opcode::RESUME);
        let d = payload.d.unwrap();
        assert_eq!(d["token"], "test-token");
        assert_eq!(d["session_id"], "s1");
        assert_eq!(d["seq"], 55);
    }

    // ── resume_or_reconnect Tests ───────────────────────────

    #[test]
    fn resume_or_reconnect_with_session() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        proxy.state.session_id = Some("s1".into());
        proxy.state.resume_gateway_url = Some("wss://gw.discord.gg".into());

        assert!(matches!(proxy.resume_or_reconnect(), LoopAction::Resume));
    }

    #[test]
    fn resume_or_reconnect_without_session() {
        let bus = EventBus::new();
        let proxy = test_proxy(&bus);

        assert!(matches!(proxy.resume_or_reconnect(), LoopAction::Reconnect));
    }

    // ── handle_loop_action Tests ────────────────────────────

    #[tokio::test]
    async fn handle_loop_action_shutdown() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let mut backoff = Backoff::new(100, 1000);
        let mut attempt = 0u32;

        let result = proxy
            .handle_loop_action(LoopAction::Shutdown, &mut backoff, &mut attempt)
            .await;

        assert!(matches!(result, Err(DiscordProxyError::Shutdown)));
    }

    #[tokio::test]
    async fn handle_loop_action_connected_resets_backoff() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let mut backoff = Backoff::new(100, 1000);
        let mut attempt = 5u32;
        // Advance backoff a few steps.
        for _ in 0..3 {
            let _ = backoff.next_delay();
        }

        let result = proxy
            .handle_loop_action(LoopAction::Connected, &mut backoff, &mut attempt)
            .await;

        assert!(result.is_ok());
        assert_eq!(attempt, 0);
    }

    #[tokio::test]
    async fn handle_loop_action_reconnect_exceeds_max_attempts() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        proxy.config.max_reconnect_attempts = 2;
        let mut backoff = Backoff::new(0, 0);
        let mut attempt = 2u32;

        let result = proxy
            .handle_loop_action(LoopAction::Reconnect, &mut backoff, &mut attempt)
            .await;

        assert!(matches!(result, Err(DiscordProxyError::Protocol(_))));
    }

    // ── handle_loop_error Tests ─────────────────────────────

    #[tokio::test]
    async fn handle_loop_error_fatal_auth() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let mut backoff = Backoff::new(0, 0);
        let mut attempt = 0u32;

        let result = proxy
            .handle_loop_error(
                DiscordProxyError::AuthenticationFailed,
                &mut backoff,
                &mut attempt,
            )
            .await;

        assert!(matches!(
            result,
            Err(DiscordProxyError::AuthenticationFailed)
        ));
    }

    #[tokio::test]
    async fn handle_loop_error_transient_triggers_reconnect() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);
        let mut backoff = Backoff::new(0, 0);
        let mut attempt = 0u32;

        let result = proxy
            .handle_loop_error(DiscordProxyError::HelloTimeout, &mut backoff, &mut attempt)
            .await;

        assert!(result.is_ok());
        assert_eq!(attempt, 1);
    }

    // ── Heartbeat Zombie Detection Tests ────────────────────

    #[tokio::test]
    async fn heartbeat_detects_zombie_when_no_ack() {
        let (ws_tx, _ws_rx) = mpsc::channel(64);
        let (zombie_tx, zombie_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let sequence = Arc::new(Mutex::new(Some(1u64)));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));

        // Mark last ACK as not received (simulates missing ACK).
        hb_state.lock().await.last_ack_received = false;

        let handle = tokio::spawn(async move {
            heartbeat::run_heartbeat(
                50, // 50ms interval for fast test
                sequence,
                hb_state,
                ws_tx,
                zombie_tx,
                shutdown_rx,
            )
            .await;
        });

        // The heartbeat should detect zombie within the first beat.
        let result = tokio::time::timeout(Duration::from_secs(2), zombie_rx).await;

        assert!(result.is_ok(), "Zombie should be detected");
        drop(shutdown_tx);
        let _ = handle.await;
    }

    #[tokio::test]
    async fn heartbeat_sends_heartbeat_on_healthy_connection() {
        let (ws_tx, mut ws_rx) = mpsc::channel(64);
        let (zombie_tx, _zombie_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let sequence = Arc::new(Mutex::new(Some(7u64)));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));

        let handle = tokio::spawn(async move {
            heartbeat::run_heartbeat(50, sequence, hb_state, ws_tx, zombie_tx, shutdown_rx).await;
        });

        // Wait for a heartbeat to arrive.
        let received = tokio::time::timeout(Duration::from_secs(2), ws_rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.op, protocol::opcode::HEARTBEAT);
        assert_eq!(received.d, Some(serde_json::Value::from(7)));

        // Shut down.
        drop(shutdown_tx);
        let _ = handle.await;
    }

    #[tokio::test]
    async fn heartbeat_shuts_down_on_signal() {
        let (ws_tx, _ws_rx) = mpsc::channel(64);
        let (zombie_tx, _zombie_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let sequence = Arc::new(Mutex::new(None));
        let hb_state = Arc::new(Mutex::new(HeartbeatState::new()));

        let handle = tokio::spawn(async move {
            heartbeat::run_heartbeat(
                60_000, // Long interval so it won't fire.
                sequence,
                hb_state,
                ws_tx,
                zombie_tx,
                shutdown_rx,
            )
            .await;
        });

        // Send shutdown immediately.
        drop(shutdown_tx);

        // Should exit quickly.
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(result.is_ok(), "Heartbeat should exit on shutdown");
    }

    // ── sleep_or_shutdown Tests ─────────────────────────────

    #[tokio::test]
    async fn sleep_or_shutdown_returns_on_shutdown() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        // Drop the shutdown_tx side to trigger shutdown.
        // We need a custom setup for this.
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        proxy.shutdown_rx = shutdown_rx;
        proxy.shutdown_tx = shutdown_tx.clone();

        // Send shutdown signal.
        let _ = shutdown_tx.send(());

        let result = proxy.sleep_or_shutdown(Duration::from_secs(60)).await;

        assert!(matches!(result, Err(DiscordProxyError::Shutdown)));
    }

    #[tokio::test]
    async fn sleep_or_shutdown_completes_after_duration() {
        let bus = EventBus::new();
        let mut proxy = test_proxy(&bus);

        let result = proxy.sleep_or_shutdown(Duration::from_millis(10)).await;

        assert!(result.is_ok());
    }
}
