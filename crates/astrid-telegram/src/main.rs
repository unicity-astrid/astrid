//! Astrid Telegram Bot — standalone binary mode.
//!
//! Connects to a running `astridd` daemon via `WebSocket` JSON-RPC and
//! exposes the agent through a Telegram bot interface.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,astrid_telegram=info")),
        )
        .init();

    let config = astrid_telegram::config::TelegramConfig::load(None)?;
    Box::pin(astrid_telegram::bot::run(config)).await
}
