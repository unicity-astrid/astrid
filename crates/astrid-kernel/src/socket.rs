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

/// Maximum byte length for a Unix domain socket path.
/// macOS/FreeBSD/OpenBSD `sockaddr_un.sun_path` is 104 bytes; Linux is 108.
#[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd"))]
const MAX_SOCKET_PATH_LEN: usize = 104;
#[cfg(not(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd")))]
const MAX_SOCKET_PATH_LEN: usize = 108;

/// Binds a local Unix Domain Socket for the OS.
/// Returns the bound listener so it can be passed into the WASM execution context.
///
/// # Errors
/// Returns an error if the socket cannot be bound, the path exceeds the
/// platform's `sun_path` limit, or another kernel instance is already
/// listening on the socket.
pub(crate) fn bind_session_socket() -> Result<UnixListener, std::io::Error> {
    let path = kernel_socket_path();

    prepare_socket_path(&path)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            std::io::Error::other(format!(
                "Failed to create socket parent directory {}: {e}",
                parent.display()
            ))
        })?;

        // Enforce 0o700 on the sessions directory. AstridHome::ensure() does
        // this at boot, but if the directory was just created by create_dir_all
        // it inherits the process umask (commonly 0o755, making the socket
        // listable by other users).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }

    UnixListener::bind(&path)
}

/// Generate a random session token and write it to the token file.
///
/// Returns both the token and the path it was written to. The caller should
/// store the path so that the exact same path is used for cleanup at shutdown
/// (avoids fallback mismatch if the env changes between boot and shutdown).
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
pub(crate) fn generate_session_token() -> Result<(SessionToken, PathBuf), std::io::Error> {
    use astrid_core::dirs::AstridHome;

    let token = SessionToken::generate();

    let home = AstridHome::resolve().map_err(|e| {
        std::io::Error::other(format!(
            "Cannot generate session token: failed to resolve ASTRID_HOME: {e}"
        ))
    })?;

    let path = home.token_path();
    token.write_to_file(&path)?;
    Ok((token, path))
}

/// Validate a socket path and handle stale/live socket detection.
///
/// Extracted from `bind_session_socket` for testability. Returns `Ok(())`
/// if the path is safe to bind (stale socket removed or no socket exists).
/// Returns `Err` if the path is too long or another kernel is listening.
fn prepare_socket_path(path: &std::path::Path) -> Result<(), std::io::Error> {
    let path_len = path.as_os_str().as_encoded_bytes().len();
    if path_len >= MAX_SOCKET_PATH_LEN {
        return Err(std::io::Error::other(format!(
            "Socket path is {path_len} bytes, exceeding the platform limit of {MAX_SOCKET_PATH_LEN} bytes: {}",
            path.display()
        )));
    }

    if path.is_symlink() {
        warn!(path = %path.display(), "Removing unexpected symlink at socket path");
        std::fs::remove_file(path).map_err(|e| {
            std::io::Error::other(format!(
                "Failed to remove symlink at socket path {}: {e}",
                path.display()
            ))
        })?;
    } else if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_stream) => {
                return Err(std::io::Error::other(format!(
                    "Another kernel instance is already running on this socket: {}",
                    path.display()
                )));
            },
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                // No listener attached: stale socket, safe to remove.
                std::fs::remove_file(path).map_err(|e| {
                    std::io::Error::other(format!(
                        "Failed to remove stale socket {}: {e}",
                        path.display()
                    ))
                })?;
            },
            Err(e) => {
                // Other errors (EACCES, etc.) may indicate a live kernel
                // under a different user or transient issue. Don't delete.
                return Err(std::io::Error::other(format!(
                    "Failed to probe existing socket {}: {e}",
                    path.display()
                )));
            },
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_too_long_is_rejected() {
        // Build a path that exceeds the platform limit.
        let long_name = "a".repeat(MAX_SOCKET_PATH_LEN + 10);
        let path = PathBuf::from(format!("/tmp/{long_name}.sock"));
        let err = prepare_socket_path(&path).unwrap_err();
        assert!(
            err.to_string().contains("exceeding the platform limit"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn stale_socket_is_removed() {
        // Bind a listener, drop it (making the socket stale), then verify
        // prepare_socket_path removes it.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");

        // Create and immediately drop a listener to leave a stale socket file.
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        drop(_listener);

        assert!(sock.exists(), "socket file should exist after bind");
        prepare_socket_path(&sock).unwrap();
        assert!(!sock.exists(), "stale socket should have been removed");
    }

    #[test]
    fn live_socket_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");

        // Keep the listener alive so connect succeeds.
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();

        let err = prepare_socket_path(&sock).unwrap_err();
        assert!(
            err.to_string().contains("already running"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn symlink_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, "not a socket").unwrap();

        let sock = dir.path().join("test.sock");
        std::os::unix::fs::symlink(&target, &sock).unwrap();
        assert!(sock.is_symlink());

        prepare_socket_path(&sock).unwrap();
        assert!(!sock.exists(), "symlink should have been removed");
        assert!(target.exists(), "target should be untouched");
    }

    #[test]
    fn nonexistent_path_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("does_not_exist.sock");
        prepare_socket_path(&sock).unwrap();
    }
}
