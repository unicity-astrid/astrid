//! IPC types — re-exported from `astrid-types` with runtime additions.

// Re-export everything from astrid-types::ipc
pub use astrid_types::ipc::*;

pub use crate::rate_limiter::IpcRateLimiter;
