use std::path::PathBuf;
use tokio::net::UnixListener;
use tracing::warn;

/// Path to the local Unix Domain Socket for the kernel.
#[must_use]
pub fn kernel_socket_path() -> PathBuf {
    use astrid_core::dirs::AstridHome;
    match AstridHome::resolve() {
        Ok(home) => home.socket_path(),
        Err(e) => {
            warn!(error = %e, "Failed to resolve ASTRID_HOME; falling back to /tmp/.astrid/sessions for unix socket");
            PathBuf::from("/tmp/.astrid/sessions/system.sock")
        },
    }
}

/// Binds a local Unix Domain Socket for the OS.
/// Returns the bound listener so it can be passed into the WASM execution context.
///
/// # Errors
/// Returns an error if the socket cannot be bound.
pub fn bind_session_socket() -> Result<UnixListener, std::io::Error> {
    let path = kernel_socket_path();

    // Remove stale socket file if it exists
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    UnixListener::bind(&path)
}
