//! Error types for the Telegram bot.

use thiserror::Error;

/// Errors produced by the Telegram bot.
#[derive(Debug, Error)]
pub enum TelegramBotError {
    /// Daemon connection failed.
    #[error("daemon connection failed: {0}")]
    DaemonConnection(String),

    /// Daemon RPC call failed.
    #[error("daemon RPC error: {0}")]
    DaemonRpc(String),

    /// Session not found for the given chat.
    #[error("no session for chat {0}")]
    #[allow(dead_code)]
    NoSession(i64),

    /// A turn is already in progress for this chat.
    #[error("turn already in progress for chat {0}")]
    #[allow(dead_code)]
    TurnInProgress(i64),

    /// Telegram API error.
    #[error("telegram API error: {0}")]
    #[allow(dead_code)]
    Telegram(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),
}

/// Convenience alias.
pub type TelegramResult<T> = Result<T, TelegramBotError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_daemon_connection() {
        let err = TelegramBotError::DaemonConnection("refused".to_string());
        assert_eq!(err.to_string(), "daemon connection failed: refused");
    }

    #[test]
    fn error_display_daemon_rpc() {
        let err = TelegramBotError::DaemonRpc("timeout".to_string());
        assert_eq!(err.to_string(), "daemon RPC error: timeout");
    }

    #[test]
    fn error_display_no_session() {
        let err = TelegramBotError::NoSession(42);
        assert_eq!(err.to_string(), "no session for chat 42");
    }

    #[test]
    fn error_display_turn_in_progress() {
        let err = TelegramBotError::TurnInProgress(99);
        assert_eq!(err.to_string(), "turn already in progress for chat 99");
    }

    #[test]
    fn error_display_telegram() {
        let err = TelegramBotError::Telegram("rate limited".to_string());
        assert_eq!(err.to_string(), "telegram API error: rate limited");
    }

    #[test]
    fn error_display_config() {
        let err = TelegramBotError::Config("missing token".to_string());
        assert_eq!(err.to_string(), "configuration error: missing token");
    }

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TelegramBotError>();
    }
}
