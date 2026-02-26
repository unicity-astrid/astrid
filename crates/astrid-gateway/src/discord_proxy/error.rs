//! Error types for the Discord Gateway proxy.

/// Errors produced by the Discord Gateway proxy.
#[derive(Debug, thiserror::Error)]
pub enum DiscordProxyError {
    /// `WebSocket` transport error.
    #[error("WebSocket error: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),

    /// HTTP error fetching the gateway URL.
    #[error("HTTP error fetching gateway URL: {0}")]
    Http(#[from] reqwest::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// `WebSocket` connection closed with a code.
    #[error("Connection closed with code {0}")]
    Closed(u16),

    /// Authentication failed (close code 4004).
    #[error("Authentication failed (close code 4004)")]
    AuthenticationFailed,

    /// Invalid intents configuration (close code 4013 or 4014).
    #[error("Invalid intents configuration (close code {0})")]
    InvalidIntents(u16),

    /// Unrecoverable close code from Discord.
    #[error("Unrecoverable close code: {0}")]
    UnrecoverableClose(u16),

    /// Shutdown was requested by the daemon.
    #[error("Shutdown requested")]
    Shutdown,

    /// The Gateway did not send a Hello payload in time.
    #[error("Timed out waiting for Hello from Gateway")]
    HelloTimeout,

    /// Protocol violation from the Gateway.
    #[error("Protocol error: {0}")]
    Protocol(String),
}

impl From<tokio_tungstenite::tungstenite::Error> for DiscordProxyError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        let err = DiscordProxyError::AuthenticationFailed;
        assert!(err.to_string().contains("4004"));

        let err = DiscordProxyError::InvalidIntents(4013);
        assert!(err.to_string().contains("4013"));

        let err = DiscordProxyError::Shutdown;
        assert!(err.to_string().contains("Shutdown"));

        let err = DiscordProxyError::HelloTimeout;
        assert!(err.to_string().contains("Hello"));

        let err = DiscordProxyError::Protocol("bad opcode".into());
        assert!(err.to_string().contains("bad opcode"));
    }

    #[test]
    fn closed_error_carries_code() {
        let err = DiscordProxyError::Closed(4001);
        assert!(err.to_string().contains("4001"));
    }
}
