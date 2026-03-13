//! Safe Rust wrappers around the Astrid OS System API (The Airlocks).
//!
//! This crate provides the idiomatic, safe Developer Experience for building
//! Pure WASM Capsules for the Astrid Microkernel. It wraps the raw, purely binary
//! FFI imports found in `astrid-sys` and provides zero-friction serialization.

#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
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

    /// Block until a message arrives on a subscription handle, or timeout.
    ///
    /// Returns the message envelope (same format as `poll_bytes`), or an
    /// empty-messages envelope if the timeout expires with no messages.
    /// Max timeout is capped at 60 000 ms by the host.
    pub fn recv_bytes(handle: impl AsRef<[u8]>, timeout_ms: u64) -> Result<Vec<u8>, SysError> {
        let timeout_str = timeout_ms.to_string();
        let message_bytes =
            unsafe { astrid_ipc_recv(handle.as_ref().to_vec(), timeout_str.into_bytes())? };
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

    /// Delete a key from the KV store.
    ///
    /// This is idempotent: deleting a non-existent key succeeds silently.
    /// The underlying store returns whether the key existed, but that
    /// information is not surfaced through the WASM host boundary.
    pub fn delete(key: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_kv_delete(key.as_ref().to_vec())? };
        Ok(())
    }

    /// List all keys matching a prefix.
    ///
    /// Returns an empty vec if no keys match. The prefix is matched
    /// against key names within the capsule's scoped namespace.
    pub fn list_keys(prefix: impl AsRef<[u8]>) -> Result<Vec<String>, SysError> {
        let result = unsafe { astrid_kv_list_keys(prefix.as_ref().to_vec())? };
        let keys: Vec<String> = serde_json::from_slice(&result)?;
        Ok(keys)
    }

    /// Delete all keys matching a prefix.
    ///
    /// Returns the number of keys deleted. The prefix is matched
    /// against key names within the capsule's scoped namespace.
    pub fn clear_prefix(prefix: impl AsRef<[u8]>) -> Result<u64, SysError> {
        let result = unsafe { astrid_kv_clear_prefix(prefix.as_ref().to_vec())? };
        let count: u64 = serde_json::from_slice(&result)?;
        Ok(count)
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

    /// Well-known config key for the kernel's Unix domain socket path.
    ///
    /// Injected automatically by the kernel into every capsule's config.
    /// Capsules that need to accept CLI connections should use
    /// [`socket_path()`] instead of hardcoding paths.
    pub const CONFIG_SOCKET_PATH: &str = "ASTRID_SOCKET_PATH";

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

    /// Returns the kernel's Unix domain socket path.
    ///
    /// Reads from the well-known `ASTRID_SOCKET_PATH` config key that the
    /// kernel injects into every capsule at load time.
    pub fn socket_path() -> Result<String, SysError> {
        let raw = get_config_string(CONFIG_SOCKET_PATH)?;
        // get_config_string returns JSON-encoded values (quoted strings).
        // Use proper JSON parsing to handle escape sequences correctly.
        let path = serde_json::from_str::<String>(raw.trim()).or_else(|_| {
            // Fallback: if the value isn't valid JSON, use it raw.
            if raw.is_empty() {
                Err(SysError::ApiError(
                    "ASTRID_SOCKET_PATH config key is empty".to_string(),
                ))
            } else {
                Ok(raw)
            }
        })?;
        // Reject paths with null bytes — they would silently truncate at the OS level.
        if path.contains('\0') {
            return Err(SysError::ApiError(
                "ASTRID_SOCKET_PATH contains null byte".to_string(),
            ));
        }
        Ok(path)
    }

    /// Signal that the capsule's run loop is ready.
    ///
    /// Call this after setting up IPC subscriptions in `run()` to let the
    /// kernel know this capsule is ready to receive events. The kernel waits
    /// for this signal before loading dependent capsules.
    pub fn signal_ready() -> Result<(), SysError> {
        unsafe { astrid_signal_ready()? };
        Ok(())
    }

    /// Retrieves the caller context (User ID and Session ID) for the current execution.
    pub fn get_caller() -> Result<crate::types::CallerContext, SysError> {
        let bytes = unsafe { astrid_get_caller()? };
        serde_json::from_slice(&bytes)
            .map_err(|e| SysError::ApiError(format!("failed to parse caller context: {}", e)))
    }

    /// Returns the current wall-clock time as milliseconds since the UNIX epoch.
    ///
    /// This is a host call - the WASM guest has no direct access to system time.
    /// Returns 0 if the host clock is unavailable.
    pub fn clock_ms() -> Result<u64, SysError> {
        let bytes = unsafe { astrid_clock_ms()? };
        let s = String::from_utf8_lossy(&bytes);
        s.trim()
            .parse::<u64>()
            .map_err(|e| SysError::ApiError(format!("clock_ms parse error: {e}")))
    }
}

/// The Hooks Airlock — Executing User Middleware
pub mod hooks {
    use super::*;

    pub fn trigger(event_bytes: &[u8]) -> Result<Vec<u8>, SysError> {
        unsafe { Ok(astrid_trigger_hook(event_bytes.to_vec())?) }
    }
}

pub mod net;
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

/// The Elicit Airlock - User Input During Install/Upgrade Lifecycle
///
/// These functions are only callable during `#[astrid::install]` and
/// `#[astrid::upgrade]` hooks. Calling them from a tool or interceptor
/// returns a host error.
pub mod elicit {
    use super::*;

    /// Internal request structure sent to the `astrid_elicit` host function.
    #[derive(Serialize)]
    struct ElicitRequest<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        key: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        options: Option<&'a [&'a str]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        default: Option<&'a str>,
    }

    /// Validates that the elicit key is non-empty and not whitespace-only.
    fn validate_key(key: &str) -> Result<(), SysError> {
        if key.trim().is_empty() {
            return Err(SysError::ApiError("elicit key must not be empty".into()));
        }
        Ok(())
    }

    /// Store a secret via the kernel's `SecretStore`. The capsule **never**
    /// receives the value. Returns `Ok(())` confirming the user provided it.
    pub fn secret(key: &str, description: &str) -> Result<(), SysError> {
        validate_key(key)?;
        let req = ElicitRequest {
            kind: "secret",
            key,
            description: Some(description),
            options: None,
            default: None,
        };
        let req_bytes = serde_json::to_vec(&req)?;
        // SAFETY: FFI call to Extism host function. The host validates the
        // request and returns a well-formed JSON response or an Extism error.
        let resp_bytes = unsafe { astrid_elicit(req_bytes)? };

        #[derive(serde::Deserialize)]
        struct SecretResp {
            ok: bool,
        }
        let resp: SecretResp = serde_json::from_slice(&resp_bytes)?;
        if !resp.ok {
            return Err(SysError::ApiError(
                "kernel did not confirm secret storage".into(),
            ));
        }
        Ok(())
    }

    /// Check if a secret has been configured (without reading it).
    pub fn has_secret(key: &str) -> Result<bool, SysError> {
        validate_key(key)?;
        #[derive(Serialize)]
        struct HasSecretRequest<'a> {
            key: &'a str,
        }
        let req_bytes = serde_json::to_vec(&HasSecretRequest { key })?;
        // SAFETY: FFI call to Extism host function. The host checks the
        // SecretStore and returns a JSON response or an Extism error.
        let resp_bytes = unsafe { astrid_has_secret(req_bytes)? };

        #[derive(serde::Deserialize)]
        struct ExistsResp {
            exists: bool,
        }
        let resp: ExistsResp = serde_json::from_slice(&resp_bytes)?;
        Ok(resp.exists)
    }

    /// Shared implementation for text elicitation with optional default.
    fn elicit_text(
        key: &str,
        description: &str,
        default: Option<&str>,
    ) -> Result<String, SysError> {
        validate_key(key)?;
        let req = ElicitRequest {
            kind: "text",
            key,
            description: Some(description),
            options: None,
            default,
        };
        let req_bytes = serde_json::to_vec(&req)?;
        // SAFETY: FFI call to Extism host function. The host validates the
        // request and returns a well-formed JSON response or an Extism error.
        let resp_bytes = unsafe { astrid_elicit(req_bytes)? };

        #[derive(serde::Deserialize)]
        struct TextResp {
            value: String,
        }
        let resp: TextResp = serde_json::from_slice(&resp_bytes)?;
        Ok(resp.value)
    }

    /// Prompt for a text value. Blocks until the user responds.
    /// Use [`secret()`] for sensitive data - this returns the value to the capsule.
    pub fn text(key: &str, description: &str) -> Result<String, SysError> {
        elicit_text(key, description, None)
    }

    /// Prompt with a default value pre-filled.
    pub fn text_with_default(
        key: &str,
        description: &str,
        default: &str,
    ) -> Result<String, SysError> {
        elicit_text(key, description, Some(default))
    }

    /// Prompt for a selection from a list. Returns the selected value.
    pub fn select(key: &str, description: &str, options: &[&str]) -> Result<String, SysError> {
        validate_key(key)?;
        if options.is_empty() {
            return Err(SysError::ApiError(
                "select requires at least one option".into(),
            ));
        }
        let req = ElicitRequest {
            kind: "select",
            key,
            description: Some(description),
            options: Some(options),
            default: None,
        };
        let req_bytes = serde_json::to_vec(&req)?;
        // SAFETY: FFI call to Extism host function. The host validates the
        // request and returns a well-formed JSON response or an Extism error.
        let resp_bytes = unsafe { astrid_elicit(req_bytes)? };

        #[derive(serde::Deserialize)]
        struct SelectResp {
            value: String,
        }
        let resp: SelectResp = serde_json::from_slice(&resp_bytes)?;
        if !options.iter().any(|o| *o == resp.value) {
            let truncated: String = resp.value.chars().take(64).collect();
            return Err(SysError::ApiError(format!(
                "host returned value '{truncated}' not in provided options",
            )));
        }
        Ok(resp.value)
    }

    /// Prompt for multiple text values (array input).
    pub fn array(key: &str, description: &str) -> Result<Vec<String>, SysError> {
        validate_key(key)?;
        let req = ElicitRequest {
            kind: "array",
            key,
            description: Some(description),
            options: None,
            default: None,
        };
        let req_bytes = serde_json::to_vec(&req)?;
        // SAFETY: FFI call to Extism host function. The host validates the
        // request and returns a well-formed JSON response or an Extism error.
        let resp_bytes = unsafe { astrid_elicit(req_bytes)? };

        #[derive(serde::Deserialize)]
        struct ArrayResp {
            values: Vec<String>,
        }
        let resp: ArrayResp = serde_json::from_slice(&resp_bytes)?;
        Ok(resp.values)
    }
}

/// Auto-subscribed interceptor bindings for run-loop capsules.
///
/// When a capsule declares both `run()` and `[[interceptor]]`, the runtime
/// auto-subscribes to each interceptor's topic and delivers events through
/// the IPC channel the run loop already reads from. This module provides
/// helpers to query the subscription mappings and dispatch events by action.
pub mod interceptors {
    use super::*;

    /// A single interceptor subscription binding.
    #[derive(Debug, serde::Deserialize)]
    pub struct InterceptorBinding {
        /// The IPC subscription handle ID (as bytes for use with `ipc::poll_bytes`/`ipc::recv_bytes`).
        pub handle_id: u64,
        /// The interceptor action name from the manifest.
        pub action: String,
        /// The event topic this interceptor subscribes to.
        pub topic: String,
    }

    impl InterceptorBinding {
        /// Return the handle ID as a string (for passing to `ipc::poll_bytes` / `ipc::recv_bytes`).
        #[must_use]
        pub fn handle_bytes(&self) -> Vec<u8> {
            self.handle_id.to_string().into_bytes()
        }
    }

    /// Query the runtime for auto-subscribed interceptor handles.
    ///
    /// Returns an empty vec if this capsule has no auto-subscribed interceptors
    /// (i.e. it does not have both `run()` and `[[interceptor]]`).
    pub fn bindings() -> Result<Vec<InterceptorBinding>, SysError> {
        let bytes = unsafe { astrid_get_interceptor_handles()? };
        let bindings: Vec<InterceptorBinding> = serde_json::from_slice(&bytes)?;
        Ok(bindings)
    }

    /// Poll all interceptor subscriptions and dispatch pending events.
    ///
    /// For each binding, polls its subscription handle. If messages are
    /// available, calls `handler(action, envelope_bytes)` for each one.
    /// The envelope bytes are the raw IPC poll response (JSON with
    /// `messages`, `dropped`, `lagged` fields).
    pub fn poll(
        bindings: &[InterceptorBinding],
        mut handler: impl FnMut(&str, &[u8]),
    ) -> Result<(), SysError> {
        for binding in bindings {
            let handle = binding.handle_bytes();
            let envelope = ipc::poll_bytes(&handle)?;

            // Only dispatch if there are actual messages (non-empty envelope).
            // The poll returns `{"messages":[],...}` when empty.
            if !envelope.is_empty() {
                handler(&binding.action, &envelope);
            }
        }
        Ok(())
    }
}

pub mod prelude {
    pub use crate::{
        SysError, cron, elicit, fs, hooks, http, interceptors, ipc, kv, process, sys, uplink,
    };
    pub use extism_pdk::plugin_fn;

    #[cfg(feature = "derive")]
    pub use astrid_sdk_macros::capsule;
}
