use astrid_core::SessionId;
use astrid_core::session_token::{
    HandshakeRequest, HandshakeResponse, PROTOCOL_VERSION, SessionToken,
};
use astrid_events::ipc::{IpcMessage, IpcPayload};
use astrid_events::{AstridEvent, EventMetadata};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::warn;

use anyhow::{Context, Result};

/// Path to the local Unix Domain Socket managed by the CLI Proxy Capsule.
#[must_use]
pub fn proxy_socket_path() -> std::path::PathBuf {
    use astrid_core::dirs::AstridHome;
    match AstridHome::resolve() {
        Ok(home) => home.socket_path(),
        Err(e) => {
            warn!(error = %e, "Failed to resolve ASTRID_HOME; falling back to /tmp/.astrid/sessions/system.sock");
            std::path::PathBuf::from("/tmp/.astrid/sessions/system.sock")
        },
    }
}

/// Path to the session authentication token file.
fn token_path() -> std::path::PathBuf {
    use astrid_core::dirs::AstridHome;
    match AstridHome::resolve() {
        Ok(home) => home.token_path(),
        Err(e) => {
            warn!(error = %e, "Failed to resolve ASTRID_HOME for token path");
            std::path::PathBuf::from("/tmp/.astrid/sessions/system.token")
        },
    }
}

/// A client connection to the Kernel's Unix Domain Socket.
pub struct SocketClient {
    read_half: tokio::net::unix::OwnedReadHalf,
    write_half: tokio::net::unix::OwnedWriteHalf,
    /// The unique identifier for this session.
    pub session_id: SessionId,
}

impl SocketClient {
    /// Attempt to connect to an existing session socket.
    ///
    /// Performs the authentication handshake: reads the session token from
    /// disk and sends a `HandshakeRequest` with the token and protocol
    /// version. The daemon validates the token and responds with a
    /// `HandshakeResponse`.
    ///
    /// # Errors
    /// Returns an error if the socket file does not exist, connection fails,
    /// or the handshake is rejected.
    pub async fn connect(session_id: SessionId) -> Result<Self> {
        let path = proxy_socket_path();

        if !path.exists() {
            anyhow::bail!("Global OS Socket not found at {}", path.display());
        }

        let mut stream = UnixStream::connect(&path)
            .await
            .context("Failed to connect to IPC socket")?;

        // Perform authenticated handshake before splitting the stream.
        perform_handshake(&mut stream).await?;

        let (read_half, write_half) = stream.into_split();

        Ok(Self {
            read_half,
            write_half,
            session_id,
        })
    }

    /// Read the next event from the Kernel.
    ///
    /// The CLI proxy capsule sends individual `IpcMessage` objects over the
    /// socket (not full `AstridEvent` wrappers). We deserialize the message
    /// and wrap it as `AstridEvent::Ipc` for the TUI handler.
    ///
    /// # Errors
    /// Returns an error if the message cannot be read or parsed.
    pub async fn read_event(&mut self) -> Result<Option<AstridEvent>> {
        let mut len_buf = [0u8; 4];
        if self.read_half.read_exact(&mut len_buf).await.is_err() {
            return Ok(None); // Connection closed
        }
        let len = u32::from_be_bytes(len_buf) as usize;

        if len > 50 * 1024 * 1024 {
            anyhow::bail!("Message too large from kernel: {len} bytes");
        }

        let mut payload = vec![0u8; len];
        self.read_half.read_exact(&mut payload).await?;

        // The proxy sends raw IpcMessage JSON — wrap it as AstridEvent::Ipc.
        let message = serde_json::from_slice::<IpcMessage>(&payload)?;
        Ok(Some(AstridEvent::Ipc {
            metadata: EventMetadata::new("cli_proxy"),
            message,
        }))
    }

    /// Send a user input message to the Kernel.
    ///
    /// # Errors
    /// Returns an error if the message cannot be sent.
    pub async fn send_input(&mut self, text: String) -> Result<()> {
        let payload = IpcPayload::UserInput {
            text,
            session_id: self.session_id.0.to_string(),
            context: None,
        };

        let msg = IpcMessage::new("user.v1.prompt", payload, self.session_id.0);

        self.send_message(msg).await
    }

    /// Send a raw IPC message to the Kernel.
    ///
    /// # Errors
    /// Returns an error if the message cannot be serialized or sent.
    pub async fn send_message(&mut self, msg: IpcMessage) -> Result<()> {
        let bytes = serde_json::to_vec(&msg)?;
        let len =
            u32::try_from(bytes.len()).context("IPC message too large (exceeds 4 GiB limit)")?;

        self.write_half.write_all(&len.to_be_bytes()).await?;
        self.write_half.write_all(&bytes).await?;
        self.write_half.flush().await?;
        Ok(())
    }
}

/// Send the authentication handshake to the daemon and validate the response.
async fn perform_handshake(stream: &mut UnixStream) -> Result<()> {
    // Read the session token from disk (fresh on every connect, no caching).
    let tok_path = token_path();
    let token = SessionToken::read_from_file(&tok_path).with_context(|| {
        format!(
            "Failed to read session token from {}. Is the daemon running?",
            tok_path.display()
        )
    })?;

    let request = HandshakeRequest {
        token: token.to_hex(),
        protocol_version: PROTOCOL_VERSION,
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    // Send as length-prefixed JSON (same wire format as IpcMessage).
    let request_bytes =
        serde_json::to_vec(&request).context("Failed to serialize handshake request")?;
    let len = u32::try_from(request_bytes.len()).context("Handshake request too large")?;

    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&request_bytes).await?;
    stream.flush().await?;

    // Read the response.
    let mut len_buf = [0u8; 4];
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        stream.read_exact(&mut len_buf),
    )
    .await
    .context("Handshake response timed out")?
    .context("Failed to read handshake response length")?;

    let resp_len = u32::from_be_bytes(len_buf) as usize;
    if resp_len > 4096 {
        anyhow::bail!("Handshake response too large: {resp_len} bytes");
    }

    let mut resp_payload = vec![0u8; resp_len];
    stream
        .read_exact(&mut resp_payload)
        .await
        .context("Failed to read handshake response payload")?;

    let response: HandshakeResponse =
        serde_json::from_slice(&resp_payload).context("Failed to parse handshake response")?;

    if response.status != "ok" {
        let reason = response
            .reason
            .unwrap_or_else(|| "unknown error".to_string());
        anyhow::bail!("Daemon rejected connection: {reason}");
    }

    Ok(())
}
