use super::*;
use serde::{Deserialize, Serialize};

/// Represents a bound network listener.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenerHandle(pub String);

/// Represents an open network stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamHandle(pub String);

/// Bind a Unix Domain Socket to the given path and return a listener handle.
pub fn bind_unix(path: impl AsRef<[u8]>) -> Result<ListenerHandle, SysError> {
    let bytes = unsafe { astrid_net_bind_unix(path.as_ref().to_vec())? };
    let handle_str = String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))?;
    Ok(ListenerHandle(handle_str))
}

/// Accept the next incoming connection on the given listener.
pub fn accept(listener: &ListenerHandle) -> Result<StreamHandle, SysError> {
    let bytes = unsafe { astrid_net_accept(listener.0.as_bytes().to_vec())? };
    let handle_str = String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))?;
    Ok(StreamHandle(handle_str))
}

/// Read bytes from the stream.
pub fn read(stream: &StreamHandle) -> Result<Vec<u8>, SysError> {
    unsafe { Ok(astrid_net_read(stream.0.as_bytes().to_vec())?) }
}

/// Write bytes to the stream.
pub fn write(stream: &StreamHandle, data: &[u8]) -> Result<(), SysError> {
    unsafe { astrid_net_write(stream.0.as_bytes().to_vec(), data.to_vec())? };
    Ok(())
}

/// Close an open stream, releasing its resources on the host.
///
/// Idempotent - closing an already-closed handle is a no-op.
pub fn close(stream: &StreamHandle) -> Result<(), SysError> {
    unsafe { astrid_net_close_stream(stream.0.as_bytes().to_vec())? };
    Ok(())
}

/// Non-blocking accept. Returns `Ok(Some(stream))` if a connection was
/// pending, `Ok(None)` if no connection is ready yet, or `Err` on
/// listener error.
pub fn poll_accept(listener: &ListenerHandle) -> Result<Option<StreamHandle>, SysError> {
    let bytes = unsafe { astrid_net_poll_accept(listener.0.as_bytes().to_vec())? };
    if bytes.is_empty() {
        return Ok(None);
    }
    let handle_str = String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))?;
    Ok(Some(StreamHandle(handle_str)))
}
