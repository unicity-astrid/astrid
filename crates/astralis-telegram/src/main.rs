//! Astralis Telegram Bot — standalone binary mode.
//!
//! Connects to a running `astralisd` daemon via `WebSocket` JSON-RPC and
//! exposes the agent through a Telegram bot interface.

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,astralis_telegram=info")),
        )
        .init();

    let config = astralis_telegram::config::TelegramConfig::load(None)?;
    Box::pin(astralis_telegram::bot::run(config)).await
}
