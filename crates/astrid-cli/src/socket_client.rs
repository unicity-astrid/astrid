use tokio::net::UnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use astrid_events::ipc::{IpcMessage, IpcPayload};
use astrid_events::AstridEvent;
use astrid_core::SessionId;
use tracing::warn;

use anyhow::{Context, Result};

/// Path to the local Unix Domain Socket managed by the CLI Proxy Capsule.
#[must_use]
pub fn proxy_socket_path(session_id: &SessionId) -> std::path::PathBuf {
    use astrid_core::dirs::AstridHome;
    let base = match AstridHome::resolve() {
        Ok(home) => home.sessions_dir(),
        Err(e) => {
            warn!(error = %e, "Failed to resolve ASTRID_HOME; falling back to /tmp/.astrid/sessions for unix socket");
            std::path::PathBuf::from("/tmp/.astrid/sessions")
        }
    };
    base.join(session_id.0.to_string()).join("ipc.sock")
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
    /// # Errors
    /// Returns an error if the socket file does not exist or connection fails.
    pub async fn connect(session_id: SessionId) -> Result<Self> {
        let path = proxy_socket_path(&session_id);

        if !path.exists() {
            anyhow::bail!("Kernel socket for session {session_id} not found at {}", path.display());
        }

        let stream = UnixStream::connect(&path).await.context("Failed to connect to IPC socket")?;
        let (read_half, write_half) = stream.into_split();

        Ok(Self {
            read_half,
            write_half,
            session_id,
        })
    }

    /// Read the next event from the Kernel.
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

        let event = serde_json::from_slice::<AstridEvent>(&payload)?;
        Ok(Some(event))
    }

    /// Send a user input message to the Kernel.
    ///
    /// # Errors
    /// Returns an error if the message cannot be sent.
    pub async fn send_input(&mut self, text: String) -> Result<()> {
        let payload = IpcPayload::UserInput {
            text,
            context: None,
        };
        
        let msg = IpcMessage::new(
            "user.prompt",
            payload,
            self.session_id.0,
        );

        self.send_message(msg).await
    }
    
    /// Send a raw IPC message to the Kernel.
    ///
    /// # Errors
    /// Returns an error if the message cannot be serialized or sent.
    #[allow(clippy::cast_possible_truncation)]
    pub async fn send_message(&mut self, msg: IpcMessage) -> Result<()> {
        let bytes = serde_json::to_vec(&msg)?;
        let len = bytes.len() as u32; 
        
        self.write_half.write_all(&len.to_be_bytes()).await?;
        self.write_half.write_all(&bytes).await?;
        Ok(())
    }
}