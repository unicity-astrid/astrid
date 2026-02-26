//! Common error types shared across frontend crates.

use thiserror::Error;

/// Errors common to all frontend crates.
///
/// Platform-specific errors (e.g., `TelegramBotError`, `DiscordBotError`)
/// compose this type via `#[from]` for transparent `?` propagation.
#[derive(Debug, Error)]
pub enum FrontendCommonError {
    /// Daemon connection failed.
    #[error("daemon connection failed: {0}")]
    DaemonConnection(String),

    /// Daemon RPC call failed.
    #[error("daemon RPC error: {0}")]
    DaemonRpc(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),
}

/// Convenience alias for results using [`FrontendCommonError`].
pub type FrontendCommonResult<T> = Result<T, FrontendCommonError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_daemon_connection() {
        let err = FrontendCommonError::DaemonConnection("refused".to_string());
        assert_eq!(err.to_string(), "daemon connection failed: refused");
    }

    #[test]
    fn error_display_daemon_rpc() {
        let err = FrontendCommonError::DaemonRpc("timeout".to_string());
        assert_eq!(err.to_string(), "daemon RPC error: timeout");
    }

    #[test]
    fn error_display_config() {
        let err = FrontendCommonError::Config("missing token".to_string());
        assert_eq!(err.to_string(), "configuration error: missing token");
    }

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FrontendCommonError>();
    }
}
