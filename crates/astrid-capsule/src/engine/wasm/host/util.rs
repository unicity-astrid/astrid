//! Utility functions for WASM host implementations.

use extism::{CurrentPlugin, Error, Val};

/// Maximum allowed length for a guest string or payload (10 MB).
pub const MAX_GUEST_PAYLOAD_LEN: u64 = 10 * 1024 * 1024;

/// Maximum allowed length for file paths (4 KB).
pub const MAX_PATH_LEN: u64 = 4 * 1024;

/// Maximum allowed length for log messages (64 KB).
pub const MAX_LOG_MESSAGE_LEN: u64 = 64 * 1024;

/// Maximum allowed length for keys (4 KB).
pub const MAX_KEY_LEN: u64 = 4 * 1024;

/// Extract a string from guest memory safely by enforcing a length limit before allocation.
///
/// # Errors
/// Returns an error if the value is not a valid pointer or if the memory allocation
/// exceeds the specified limit.
#[allow(clippy::cast_sign_loss)]
pub fn get_safe_string(plugin: &mut CurrentPlugin, val: &Val, limit: u64) -> Result<String, Error> {
    let ptr = match val {
        Val::I64(v) => *v as u64,
        Val::I32(v) => u64::from(*v as u32),
        _ => return Err(Error::msg("expected memory pointer value")),
    };

    let len = plugin.memory_length(ptr)?;
    if len > limit {
        return Err(Error::msg(format!(
            "memory allocation of {len} bytes exceeds maximum allowed limit of {limit} bytes"
        )));
    }

    #[allow(clippy::cast_possible_wrap)]
    let safe_val = Val::I64(ptr as i64);
    plugin.memory_get_val(&safe_val)
}

/// Extract raw bytes from guest memory safely by enforcing a length limit before allocation.
///
/// # Errors
/// Returns an error if the value is not a valid pointer or if the memory allocation
/// exceeds the specified limit.
#[allow(clippy::cast_sign_loss)]
pub fn get_safe_bytes(plugin: &mut CurrentPlugin, val: &Val, limit: u64) -> Result<Vec<u8>, Error> {
    let ptr = match val {
        Val::I64(v) => *v as u64,
        Val::I32(v) => u64::from(*v as u32),
        _ => return Err(Error::msg("expected memory pointer value")),
    };

    let len = plugin.memory_length(ptr)?;
    if len > limit {
        return Err(Error::msg(format!(
            "memory allocation of {len} bytes exceeds maximum allowed limit of {limit} bytes"
        )));
    }

    #[allow(clippy::cast_possible_wrap)]
    let safe_val = Val::I64(ptr as i64);
    let memory: Vec<u8> = plugin.memory_get_val(&safe_val)?;
    Ok(memory)
}
