//! Teloxide bot setup, dispatcher, and handler registration.

use std::sync::Arc;

use teloxide::dispatching::UpdateFilterExt;
use teloxide::prelude::*;
use tracing::{info, warn};

use crate::approval::ApprovalManager;
use crate::client::DaemonClient;
use crate::config::TelegramConfig;
use crate::elicitation::ElicitationManager;
use crate::handler::{self, BotState};
use crate::session::SessionMap;

/// Build `BotState` and the teloxide handler tree from a config and daemon
/// client. Shared by both standalone and embedded entry points.
fn build_state_and_handler(
    config: TelegramConfig,
    daemon: DaemonClient,
) -> (
    BotState,
    Bot,
    teloxide::dispatching::UpdateHandler<anyhow::Error>,
) {
    if config.allowed_user_ids.is_empty() {
        warn!(
            "Telegram bot starting with NO user restrictions — \
             any Telegram user can interact with the agent. \
             Set [telegram] allowed_user_ids in config to restrict access."
        );
    }

    let bot = Bot::new(&config.bot_token);

    let state = BotState {
        daemon: Arc::new(daemon),
        sessions: SessionMap::new(),
        config: Arc::new(config),
        approvals: ApprovalManager::new(),
        elicitations: ElicitationManager::new(),
    };

    let message_handler = Update::filter_message().endpoint({
        let state = state.clone();
        move |bot: Bot, msg: Message| {
            let state = state.clone();
            async move { Box::pin(handler::handle_message(bot, msg, state)).await }
        }
    });

    let callback_handler = Update::filter_callback_query().endpoint({
        let state = state.clone();
        move |bot: Bot, query: CallbackQuery| {
            let state = state.clone();
            async move { handle_callback(bot, query, state).await }
        }
    });

    let handler = dptree::entry()
        .branch(message_handler)
        .branch(callback_handler);

    (state, bot, handler)
}

/// Run the Telegram bot until shutdown (standalone mode).
///
/// Sets up a Ctrl+C handler and connects to the daemon using the URL from
/// config (or auto-discovers from `~/.astralis/daemon.port`).
pub async fn run(config: TelegramConfig) -> anyhow::Result<()> {
    info!("Connecting to daemon...");
    let daemon = DaemonClient::connect(config.daemon_url.as_deref()).await?;
    info!("Connected to daemon");

    let (_state, bot, handler) = build_state_and_handler(config, daemon);

    info!("Starting Telegram bot...");
    Box::pin(
        Dispatcher::builder(bot, handler)
            .enable_ctrlc_handler()
            .build()
            .dispatch(),
    )
    .await;

    info!("Bot stopped");
    Ok(())
}

/// Spawn the embedded Telegram bot as a background task if `bot_token` is
/// configured and `embedded` is `true`.
///
/// Returns `None` if the bot should not be started (no token or embedded
/// disabled). The returned `JoinHandle` can be aborted on daemon shutdown.
pub fn spawn_embedded(
    telegram_cfg: &astralis_config::TelegramSection,
    addr: std::net::SocketAddr,
) -> Option<tokio::task::JoinHandle<()>> {
    if !telegram_cfg.embedded {
        return None;
    }

    let bot_token = telegram_cfg.bot_token.clone()?;

    let daemon_url = format!("ws://127.0.0.1:{}", addr.port());
    let config = TelegramConfig {
        bot_token,
        daemon_url: Some(daemon_url.clone()),
        allowed_user_ids: telegram_cfg.allowed_user_ids.clone(),
        workspace_path: telegram_cfg.workspace_path.clone(),
    };

    info!("Starting embedded Telegram bot...");

    Some(tokio::spawn(async move {
        if let Err(e) = run_embedded(&daemon_url, config).await {
            warn!(error = %e, "Embedded Telegram bot exited with error");
        }
    }))
}

/// Run the Telegram bot in embedded mode (spawned by the daemon).
///
/// Connects to the daemon at the given explicit URL. Does **not** install a
/// Ctrl+C handler — the daemon manages shutdown by aborting this task.
pub async fn run_embedded(daemon_url: &str, config: TelegramConfig) -> anyhow::Result<()> {
    info!(url = daemon_url, "Embedded bot connecting to daemon...");
    let daemon = DaemonClient::connect_url(daemon_url).await?;
    info!("Embedded bot connected to daemon");

    let (_state, bot, handler) = build_state_and_handler(config, daemon);

    info!("Starting embedded Telegram bot...");

    // No ctrlc handler — daemon aborts this task on shutdown.
    Box::pin(Dispatcher::builder(bot, handler).build().dispatch()).await;

    info!("Embedded bot stopped");
    Ok(())
}

/// Handle callback queries (approval and elicitation buttons).
async fn handle_callback(bot: Bot, query: CallbackQuery, state: BotState) -> anyhow::Result<()> {
    // Access control: verify the button-presser is on the allowlist.
    if !state.config.is_user_allowed(query.from.id.0) {
        let _ = bot
            .answer_callback_query(&query.id)
            .text("Not authorized")
            .await;
        return Ok(());
    }

    // Try approval handler first.
    if state
        .approvals
        .handle_callback(&bot, &query, &state.daemon, &state.sessions)
        .await
    {
        return Ok(());
    }

    // Try elicitation handler.
    if state
        .elicitations
        .handle_callback(&bot, &query, &state.daemon, &state.sessions)
        .await
    {
        return Ok(());
    }

    // Unknown callback.
    let _ = bot
        .answer_callback_query(&query.id)
        .text("Unknown action")
        .await;

    Ok(())
}
