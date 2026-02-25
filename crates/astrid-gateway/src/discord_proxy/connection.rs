//! `WebSocket` connection management for the Discord Gateway.
//!
//! Handles connecting, sending, receiving, and closing the `WebSocket`
//! connection to Discord's Gateway servers.

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::error::DiscordProxyError;
use super::protocol::GatewayPayload;

/// Type alias for the `WebSocket` stream used by the proxy.
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A live `WebSocket` connection to the Discord Gateway.
///
/// Wraps the split read/write halves of a `tokio-tungstenite` stream
/// and provides typed send/receive for [`GatewayPayload`].
pub(crate) struct GatewayConnection {
    /// Write half of the `WebSocket`.
    writer: SplitSink<WsStream, Message>,
    /// Read half of the `WebSocket`.
    reader: SplitStream<WsStream>,
}

impl GatewayConnection {
    /// Connect to the given Gateway URL.
    ///
    /// The URL must use the `wss://` scheme. Returns an error on
    /// connection or TLS failure.
    pub(super) async fn connect(url: &str) -> Result<Self, DiscordProxyError> {
        let (ws, _response) = connect_async(url).await?;
        let (writer, reader) = ws.split();
        Ok(Self { writer, reader })
    }

    /// Send a Gateway payload as JSON text.
    #[allow(dead_code)]
    pub(super) async fn send(&mut self, payload: &GatewayPayload) -> Result<(), DiscordProxyError> {
        let json = serde_json::to_string(payload)?;
        self.writer.send(Message::Text(json.into())).await?;
        Ok(())
    }

    /// Receive the next Gateway payload.
    ///
    /// Returns `Ok(None)` if the connection is cleanly closed.
    /// Returns the close code via [`DiscordProxyError::Closed`] if
    /// the server sends a close frame.
    #[allow(dead_code)]
    pub(super) async fn recv(&mut self) -> Result<Option<GatewayPayload>, DiscordProxyError> {
        loop {
            match self.reader.next().await {
                Some(Ok(Message::Text(text))) => {
                    let payload: GatewayPayload = serde_json::from_str(&text)?;
                    return Ok(Some(payload));
                },
                Some(Ok(Message::Close(frame))) => {
                    let code = frame.as_ref().map_or(1000, |f| f.code.into());
                    return Err(DiscordProxyError::Closed(code));
                },
                Some(Ok(
                    Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_),
                )) => {
                    // Ping/pong handled by tungstenite; binary skipped.
                },
                Some(Err(e)) => {
                    return Err(e.into());
                },
                None => {
                    // Stream ended.
                    return Ok(None);
                },
            }
        }
    }

    /// Send a close frame and shut down the connection.
    #[allow(dead_code)]
    pub(super) async fn close(&mut self, code: u16) -> Result<(), DiscordProxyError> {
        let frame = tokio_tungstenite::tungstenite::protocol::CloseFrame {
            code: code.into(),
            reason: "closing".into(),
        };
        self.writer.send(Message::Close(Some(frame))).await?;
        Ok(())
    }

    /// Take the split halves for use with `tokio::select!`.
    ///
    /// After calling this, `send`/`recv`/`close` can no longer be used.
    /// Instead, use the returned writer and reader directly.
    pub(super) fn into_parts(self) -> (SplitSink<WsStream, Message>, SplitStream<WsStream>) {
        (self.writer, self.reader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_connection_send_requires_json_serializable() {
        // Verify GatewayPayload can be serialized (compile-time check).
        let payload = GatewayPayload {
            op: 1,
            d: Some(serde_json::json!(42)),
            s: None,
            t: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"op\":1"));
    }
}
