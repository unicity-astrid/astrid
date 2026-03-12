use std::path::PathBuf;

use astrid_core::session_token::SessionToken;
use tokio::net::UnixListener;
use tracing::warn;

/// Path to the local Unix Domain Socket for the kernel.
#[must_use]
pub(crate) fn kernel_socket_path() -> PathBuf {
    use astrid_core::dirs::AstridHome;
    match AstridHome::resolve() {
        Ok(home) => home.socket_path(),
        Err(e) => {
            warn!(error = %e, "Failed to resolve ASTRID_HOME; falling back to /tmp/.astrid/sessions/system.sock");
            PathBuf::from("/tmp/.astrid/sessions/system.sock")
        },
    }
}

/// Binds a local Unix Domain Socket for the OS.
/// Returns the bound listener so it can be passed into the WASM execution context.
///
/// # Errors
/// Returns an error if the socket cannot be bound.
pub(crate) fn bind_session_socket() -> Result<UnixListener, std::io::Error> {
    let path = kernel_socket_path();

    // Remove stale socket file if it exists
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            std::io::Error::other(format!(
                "Failed to create socket parent directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    UnixListener::bind(&path)
}

/// Generate a random session token and write it to the token file.
///
/// The token is written with 0o600 permissions so only the owning user
/// can read it. The CLI reads this token at connect time and sends it
/// as part of the handshake.
///
/// # Errors
/// Returns an error if `ASTRID_HOME` cannot be resolved or the token file
/// cannot be written. Unlike socket/CLI paths, there is no `/tmp` fallback
/// because writing a secret token under a world-listable directory would
/// undermine the authentication it provides.
pub(crate) fn generate_session_token() -> Result<SessionToken, std::io::Error> {
    use astrid_core::dirs::AstridHome;

    let token = SessionToken::generate();

    let home = AstridHome::resolve().map_err(|e| {
        std::io::Error::other(format!(
            "Cannot generate session token: failed to resolve ASTRID_HOME: {e}"
        ))
    })?;

    token.write_to_file(&home.token_path())?;
    Ok(token)
}
