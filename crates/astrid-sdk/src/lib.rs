//! Safe Rust wrappers around the Astrid OS System API (The Airlocks).
//!
//! This crate provides the idiomatic, safe Developer Experience for building
//! Pure WASM Capsules for the Astrid Microkernel. It wraps the raw, purely binary
//! FFI imports found in `astrid-sys` and provides zero-friction serialization.

#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use astrid_sys::*;
use borsh::{BorshDeserialize, BorshSerialize};
pub use extism_pdk;
pub use schemars;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

/// Core error type for SDK operations
#[derive(Error, Debug)]
pub enum SysError {
    #[error("Host function call failed: {0}")]
    HostError(#[from] extism_pdk::Error),
    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("MessagePack serialization error: {0}")]
    MsgPackEncodeError(#[from] rmp_serde::encode::Error),
    #[error("MessagePack deserialization error: {0}")]
    MsgPackDecodeError(#[from] rmp_serde::decode::Error),
    #[error("Borsh serialization error: {0}")]
    BorshError(#[from] std::io::Error),
    #[error("API logic error: {0}")]
    ApiError(String),
}

/// The VFS Airlock — Interacting with the Virtual File System
pub mod fs {
    use super::*;

    pub fn exists(path: impl AsRef<[u8]>) -> Result<bool, SysError> {
        let result = unsafe { astrid_fs_exists(path.as_ref().to_vec())? };
        Ok(!result.is_empty() && result[0] != 0)
    }

    pub fn read_bytes(path: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_read_file(path.as_ref().to_vec())? };
        Ok(result)
    }

    pub fn read_string(path: impl AsRef<[u8]>) -> Result<String, SysError> {
        let bytes = read_bytes(path)?;
        String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))
    }

    pub fn write_bytes(path: impl AsRef<[u8]>, content: &[u8]) -> Result<(), SysError> {
        unsafe { astrid_write_file(path.as_ref().to_vec(), content.to_vec())? };
        Ok(())
    }

    pub fn write_string(path: impl AsRef<[u8]>, content: &str) -> Result<(), SysError> {
        write_bytes(path, content.as_bytes())
    }

    pub fn mkdir(path: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_fs_mkdir(path.as_ref().to_vec())? };
        Ok(())
    }

    pub fn readdir(path: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_fs_readdir(path.as_ref().to_vec())? };
        Ok(result)
    }

    pub fn stat(path: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_fs_stat(path.as_ref().to_vec())? };
        Ok(result)
    }

    pub fn unlink(path: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_fs_unlink(path.as_ref().to_vec())? };
        Ok(())
    }
}

/// The IPC Airlock — Communicating with the Event Bus
pub mod ipc {
    use super::*;

    pub fn publish_bytes(topic: impl AsRef<[u8]>, payload: &[u8]) -> Result<(), SysError> {
        unsafe { astrid_ipc_publish(topic.as_ref().to_vec(), payload.to_vec())? };
        Ok(())
    }

    pub fn publish_json<T: Serialize>(
        topic: impl AsRef<[u8]>,
        payload: &T,
    ) -> Result<(), SysError> {
        let bytes = serde_json::to_vec(payload)?;
        publish_bytes(topic, &bytes)
    }

    pub fn publish_msgpack<T: Serialize>(
        topic: impl AsRef<[u8]>,
        payload: &T,
    ) -> Result<(), SysError> {
        let bytes = rmp_serde::to_vec_named(payload)?;
        publish_bytes(topic, &bytes)
    }

    pub fn subscribe(topic: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let handle_bytes = unsafe { astrid_ipc_subscribe(topic.as_ref().to_vec())? };
        Ok(handle_bytes)
    }

    pub fn unsubscribe(handle: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_ipc_unsubscribe(handle.as_ref().to_vec())? };
        Ok(())
    }

    pub fn poll_bytes(handle: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let message_bytes = unsafe { astrid_ipc_poll(handle.as_ref().to_vec())? };
        Ok(message_bytes)
    }
}

/// The Uplink Airlock — Direct Frontend Messaging
pub mod uplink {
    use super::*;

    pub fn register(
        name: impl AsRef<[u8]>,
        platform: impl AsRef<[u8]>,
        profile: impl AsRef<[u8]>,
    ) -> Result<Vec<u8>, SysError> {
        let id_bytes = unsafe {
            astrid_uplink_register(
                name.as_ref().to_vec(),
                platform.as_ref().to_vec(),
                profile.as_ref().to_vec(),
            )?
        };
        Ok(id_bytes)
    }

    pub fn send_bytes(
        uplink_id: impl AsRef<[u8]>,
        platform_user_id: impl AsRef<[u8]>,
        content: &[u8],
    ) -> Result<Vec<u8>, SysError> {
        let result = unsafe {
            astrid_uplink_send(
                uplink_id.as_ref().to_vec(),
                platform_user_id.as_ref().to_vec(),
                content.to_vec(),
            )?
        };
        Ok(result)
    }
}

/// The KV Airlock — Persistent Key-Value Storage
pub mod kv {
    use super::*;

    pub fn get_bytes(key: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_kv_get(key.as_ref().to_vec())? };
        Ok(result)
    }

    pub fn set_bytes(key: impl AsRef<[u8]>, value: &[u8]) -> Result<(), SysError> {
        unsafe { astrid_kv_set(key.as_ref().to_vec(), value.to_vec())? };
        Ok(())
    }

    pub fn get_json<T: DeserializeOwned>(key: impl AsRef<[u8]>) -> Result<T, SysError> {
        let bytes = get_bytes(key)?;
        let parsed = serde_json::from_slice(&bytes)?;
        Ok(parsed)
    }

    pub fn set_json<T: Serialize>(key: impl AsRef<[u8]>, value: &T) -> Result<(), SysError> {
        let bytes = serde_json::to_vec(value)?;
        set_bytes(key, &bytes)
    }

    pub fn get_borsh<T: BorshDeserialize>(key: impl AsRef<[u8]>) -> Result<T, SysError> {
        let bytes = get_bytes(key)?;
        let parsed = borsh::from_slice(&bytes)?;
        Ok(parsed)
    }

    pub fn set_borsh<T: BorshSerialize>(key: impl AsRef<[u8]>, value: &T) -> Result<(), SysError> {
        let bytes = borsh::to_vec(value)?;
        set_bytes(key, &bytes)
    }
}

/// The HTTP Airlock — External Network Requests
pub mod http {
    use super::*;

    /// Issue a raw HTTP request. The `request_bytes` payload format depends on the Kernel's expectation
    /// (e.g. JSON or MsgPack representation of the HTTP request).
    pub fn request_bytes(request_bytes: &[u8]) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_http_request(request_bytes.to_vec())? };
        Ok(result)
    }
}

/// The Cron Airlock — Dynamic Background Scheduling
pub mod cron {
    use super::*;

    /// Schedule a dynamic cron job that will wake up this capsule.
    pub fn schedule(
        name: impl AsRef<[u8]>,
        schedule: impl AsRef<[u8]>,
        payload: &[u8],
    ) -> Result<(), SysError> {
        unsafe {
            astrid_cron_schedule(
                name.as_ref().to_vec(),
                schedule.as_ref().to_vec(),
                payload.to_vec(),
            )?
        };
        Ok(())
    }

    /// Cancel a previously scheduled dynamic cron job.
    pub fn cancel(name: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_cron_cancel(name.as_ref().to_vec())? };
        Ok(())
    }
}

/// The Sys Airlock — System logging and configuration
pub mod types;

pub mod sys {
    use super::*;

    pub fn log(level: impl AsRef<[u8]>, message: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_log(level.as_ref().to_vec(), message.as_ref().to_vec())? };
        Ok(())
    }

    pub fn get_config_bytes(key: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_get_config(key.as_ref().to_vec())? };
        Ok(result)
    }

    pub fn get_config_string(key: impl AsRef<[u8]>) -> Result<String, SysError> {
        let bytes = get_config_bytes(key)?;
        String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))
    }

    /// Retrieves the caller context (User ID and Session ID) for the current execution.
    pub fn get_caller() -> Result<crate::types::CallerContext, SysError> {
        let bytes = unsafe { astrid_get_caller()? };
        serde_json::from_slice(&bytes)
            .map_err(|e| SysError::ApiError(format!("failed to parse caller context: {}", e)))
    }
}

/// The Process Airlock — Spawning Native Host Processes
pub mod process {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Request payload for spawning a host process.
    #[derive(Debug, Serialize)]
    pub struct ProcessRequest<'a> {
        pub cmd: &'a str,
        pub args: &'a [&'a str],
    }

    /// Result returned from a spawned host process.
    #[derive(Debug, Deserialize)]
    pub struct ProcessResult {
        pub stdout: String,
        pub stderr: String,
        pub exit_code: i32,
    }

    /// Spawns a native host process.
    /// The Capsule must have the `host_process` capability granted for this command.
    pub fn spawn(cmd: &str, args: &[&str]) -> Result<ProcessResult, SysError> {
        let req = ProcessRequest { cmd, args };
        let req_bytes = serde_json::to_vec(&req)?;
        let result_bytes = unsafe { astrid_spawn_host(req_bytes)? };
        let result: ProcessResult = serde_json::from_slice(&result_bytes)?;
        Ok(result)
    }
}

pub mod prelude {
    pub use crate::{SysError, cron, fs, http, ipc, kv, process, sys, uplink};
    pub use extism_pdk::plugin_fn;

    #[cfg(feature = "derive")]
    pub use astrid_sdk_macros::capsule;
}
