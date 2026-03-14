//! Safe Rust SDK for building User-Space Capsules on Astrid OS.
//!
//! # Design Intent
//!
//! This SDK is meant to feel like using `std`. Module names, function
//! signatures, and type patterns follow Rust standard library conventions so
//! that a Rust developer's instinct for "where would I find X?" gives the
//! right answer without reading docs. When Astrid adds a concept that has no
//! `std` counterpart (IPC, capabilities, interceptors), the API still follows
//! the same style: typed handles, `Result`-based errors, and `impl AsRef`
//! parameters.
//!
//! See `docs/sdk-ergonomics.md` for the full design rationale.
//!
//! # Module Layout (mirrors `std` where applicable)
//!
//! | Module          | std equivalent   | Purpose                                |
//! |-----------------|------------------|----------------------------------------|
//! | [`fs`]          | `std::fs`        | Virtual filesystem                     |
//! | [`net`]         | `std::net`       | Unix domain sockets                    |
//! | [`process`]     | `std::process`   | Host process execution                 |
//! | [`env`]         | `std::env`       | Capsule configuration / env vars       |
//! | [`time`]        | `std::time`      | Wall-clock access                      |
//! | [`log`]         | `log` crate      | Structured logging                     |
//! | [`runtime`]     | N/A              | OS signaling and caller context        |
//! | [`ipc`]         | N/A              | Event bus messaging                    |
//! | [`kv`]          | N/A              | Persistent key-value storage           |
//! | [`http`]        | N/A              | Outbound HTTP requests                 |
//! | [`cron`]        | N/A              | Scheduled background tasks             |
//! | [`uplink`]      | N/A              | Direct frontend messaging              |
//! | [`hooks`]       | N/A              | User middleware triggers               |
//! | [`elicit`]      | N/A              | Interactive install/upgrade prompts    |
//! | [`identity`]    | N/A              | Platform user identity resolution      |
//! | [`approval`]    | N/A              | Human approval for sensitive actions   |

#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use astrid_sys::*;
use borsh::{BorshDeserialize, BorshSerialize};
// Re-exported for the #[capsule] macro's generated code. Not part of the
// public API - capsule authors should never need to import these directly.
#[doc(hidden)]
pub use extism_pdk;
#[doc(hidden)]
pub use schemars;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
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

/// Virtual filesystem (mirrors `std::fs` naming).
pub mod fs {
    use super::*;

    /// Check if a path exists. Like `std::fs::exists` (nightly).
    pub fn exists(path: impl AsRef<[u8]>) -> Result<bool, SysError> {
        let result = unsafe { astrid_fs_exists(path.as_ref().to_vec())? };
        Ok(!result.is_empty() && result[0] != 0)
    }

    /// Read the entire contents of a file as bytes. Like `std::fs::read`.
    pub fn read(path: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_read_file(path.as_ref().to_vec())? };
        Ok(result)
    }

    /// Read the entire contents of a file as a string. Like `std::fs::read_to_string`.
    pub fn read_to_string(path: impl AsRef<[u8]>) -> Result<String, SysError> {
        let bytes = read(path)?;
        String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))
    }

    /// Write bytes to a file. Like `std::fs::write`.
    pub fn write(path: impl AsRef<[u8]>, contents: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_write_file(path.as_ref().to_vec(), contents.as_ref().to_vec())? };
        Ok(())
    }

    /// Create a directory. Like `std::fs::create_dir`.
    pub fn create_dir(path: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_fs_mkdir(path.as_ref().to_vec())? };
        Ok(())
    }

    /// Read directory entries. Like `std::fs::read_dir`.
    pub fn read_dir(path: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_fs_readdir(path.as_ref().to_vec())? };
        Ok(result)
    }

    /// Get file metadata. Like `std::fs::metadata`.
    pub fn metadata(path: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_fs_stat(path.as_ref().to_vec())? };
        Ok(result)
    }

    /// Remove a file. Like `std::fs::remove_file`.
    pub fn remove_file(path: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_fs_unlink(path.as_ref().to_vec())? };
        Ok(())
    }
}

/// Event bus messaging (like `std::sync::mpsc` but topic-based).
pub mod ipc {
    use super::*;

    /// An active subscription to an IPC topic. Returned by [`subscribe`].
    ///
    /// Follows the typed-handle pattern used by [`crate::net::ListenerHandle`].
    #[derive(Debug, Clone)]
    pub struct SubscriptionHandle(pub(crate) Vec<u8>);

    impl SubscriptionHandle {
        /// Raw handle bytes for interop with lower-level APIs.
        #[must_use]
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }
    }

    // Allow existing code using `impl AsRef<[u8]>` to pass a SubscriptionHandle.
    impl AsRef<[u8]> for SubscriptionHandle {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }

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

    /// Subscribe to an IPC topic. Returns a typed handle for polling/receiving.
    pub fn subscribe(topic: impl AsRef<[u8]>) -> Result<SubscriptionHandle, SysError> {
        let handle_bytes = unsafe { astrid_ipc_subscribe(topic.as_ref().to_vec())? };
        Ok(SubscriptionHandle(handle_bytes))
    }

    pub fn unsubscribe(handle: &SubscriptionHandle) -> Result<(), SysError> {
        unsafe { astrid_ipc_unsubscribe(handle.0.clone())? };
        Ok(())
    }

    pub fn poll_bytes(handle: &SubscriptionHandle) -> Result<Vec<u8>, SysError> {
        let message_bytes = unsafe { astrid_ipc_poll(handle.0.clone())? };
        Ok(message_bytes)
    }

    /// Block until a message arrives on a subscription handle, or timeout.
    ///
    /// Returns the message envelope (same format as `poll_bytes`), or an
    /// empty-messages envelope if the timeout expires with no messages.
    /// Max timeout is capped at 60 000 ms by the host.
    pub fn recv_bytes(handle: &SubscriptionHandle, timeout_ms: u64) -> Result<Vec<u8>, SysError> {
        let timeout_str = timeout_ms.to_string();
        let message_bytes = unsafe { astrid_ipc_recv(handle.0.clone(), timeout_str.into_bytes())? };
        Ok(message_bytes)
    }
}

/// Direct frontend messaging (uplinks to CLI, Telegram, etc.).
pub mod uplink {
    use super::*;

    /// An opaque uplink connection identifier. Returned by [`register`].
    #[derive(Debug, Clone)]
    pub struct UplinkId(pub(crate) Vec<u8>);

    impl UplinkId {
        /// Raw ID bytes for interop with lower-level APIs.
        #[must_use]
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }
    }

    impl AsRef<[u8]> for UplinkId {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }

    /// Register a new uplink connection. Returns a typed [`UplinkId`].
    pub fn register(
        name: impl AsRef<[u8]>,
        platform: impl AsRef<[u8]>,
        profile: impl AsRef<[u8]>,
    ) -> Result<UplinkId, SysError> {
        let id_bytes = unsafe {
            astrid_uplink_register(
                name.as_ref().to_vec(),
                platform.as_ref().to_vec(),
                profile.as_ref().to_vec(),
            )?
        };
        Ok(UplinkId(id_bytes))
    }

    /// Send bytes to a user via an uplink.
    pub fn send_bytes(
        uplink_id: &UplinkId,
        platform_user_id: impl AsRef<[u8]>,
        content: &[u8],
    ) -> Result<Vec<u8>, SysError> {
        let result = unsafe {
            astrid_uplink_send(
                uplink_id.0.clone(),
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

    // ---- Versioned KV helpers ----

    /// Internal envelope for versioned KV data.
    ///
    /// Wire format: `{"__sv": <version>, "data": <payload>}`.
    /// The `__sv` prefix is deliberately ugly to avoid collision with
    /// user struct fields.
    #[derive(Serialize, Deserialize)]
    struct VersionedEnvelope<T> {
        #[serde(rename = "__sv")]
        schema_version: u32,
        data: T,
    }

    /// Result of reading versioned data from KV.
    #[derive(Debug)]
    pub enum Versioned<T> {
        /// Data is at the expected schema version.
        Current(T),
        /// Data is at an older version and needs migration.
        NeedsMigration {
            /// Raw JSON value of the `data` field.
            raw: serde_json::Value,
            /// The schema version that was stored.
            stored_version: u32,
        },
        /// Key exists but data has no version envelope (pre-versioning legacy data).
        Unversioned(serde_json::Value),
        /// Key does not exist in KV.
        NotFound,
    }

    /// Write versioned data to KV, wrapped in a schema-version envelope.
    ///
    /// The stored JSON looks like `{"__sv": 1, "data": { ... }}`.
    /// Use [`get_versioned`] or [`get_versioned_or_migrate`] to read it back.
    pub fn set_versioned<T: Serialize>(
        key: impl AsRef<[u8]>,
        value: &T,
        version: u32,
    ) -> Result<(), SysError> {
        let envelope = VersionedEnvelope {
            schema_version: version,
            data: value,
        };
        set_json(key, &envelope)
    }

    /// Read versioned data from KV.
    ///
    /// Returns [`Versioned::Current`] if the stored version matches
    /// `current_version`. Returns [`Versioned::NeedsMigration`] for older
    /// versions. Returns an error for versions newer than `current_version`
    /// (fail secure - don't silently interpret data from a schema you don't
    /// understand).
    ///
    /// Data written by plain [`set_json`] (no envelope) returns
    /// [`Versioned::Unversioned`].
    pub fn get_versioned<T: DeserializeOwned>(
        key: impl AsRef<[u8]>,
        current_version: u32,
    ) -> Result<Versioned<T>, SysError> {
        let bytes = get_bytes(&key)?;
        parse_versioned(&bytes, current_version)
    }

    /// Core parsing logic for versioned KV data, separated from FFI for
    /// testability. Operates on raw bytes as returned by `get_bytes`.
    fn parse_versioned<T: DeserializeOwned>(
        bytes: &[u8],
        current_version: u32,
    ) -> Result<Versioned<T>, SysError> {
        // The host function `astrid_kv_get` returns an empty slice when the
        // key is absent. A present key written via set_json/set_versioned
        // always has at least the JSON envelope bytes, so empty = not found.
        if bytes.is_empty() {
            return Ok(Versioned::NotFound);
        }

        let mut value: serde_json::Value = serde_json::from_slice(bytes)?;

        // Detect envelope by checking for __sv (u64) + data fields.
        // If __sv is present but malformed (not a number, or missing data),
        // return an error rather than silently treating as unversioned.
        let sv_field = value.get("__sv");
        let has_sv = sv_field.is_some();
        let envelope_version = sv_field.and_then(|v| v.as_u64());
        let has_data = value.get("data").is_some();

        match (has_sv, envelope_version, has_data) {
            // Valid envelope: __sv is a u64 and data is present.
            // Take ownership of the data field via remove() to avoid cloning.
            (_, Some(v), true) => {
                let v = u32::try_from(v)
                    .map_err(|_| SysError::ApiError("schema version exceeds u32::MAX".into()))?;
                // Safety: the match guard confirmed has_data=true, so
                // value is an object with a "data" key. This is infallible.
                let data = value
                    .as_object_mut()
                    .and_then(|m| m.remove("data"))
                    .expect("data field guaranteed by match condition");
                if v == current_version {
                    let parsed: T = serde_json::from_value(data)?;
                    Ok(Versioned::Current(parsed))
                } else if v < current_version {
                    Ok(Versioned::NeedsMigration {
                        raw: data,
                        stored_version: v,
                    })
                } else {
                    Err(SysError::ApiError(format!(
                        "stored schema version {v} is newer than current \
                         version {current_version} - cannot safely read"
                    )))
                }
            },
            // Malformed envelope: __sv present but data missing or __sv not a number.
            (true, _, _) => Err(SysError::ApiError(
                "malformed versioned envelope: __sv field present but \
                 data field missing or __sv is not a number"
                    .into(),
            )),
            // No __sv field at all: plain unversioned data.
            (false, _, _) => Ok(Versioned::Unversioned(value)),
        }
    }

    /// Read versioned data, automatically migrating older versions.
    ///
    /// `migrate_fn` receives the raw JSON and the stored version, and must
    /// return a `T` at `current_version`. The migrated value is automatically
    /// saved back to KV.
    ///
    /// **Warning:** The original data is overwritten after a successful
    /// migration. If the write-back fails, the original data is preserved
    /// and the migration will be re-attempted on the next call. Ensure
    /// `migrate_fn` is idempotent and correct - there is no rollback
    /// after a successful write.
    ///
    /// For [`Versioned::Unversioned`] data, `migrate_fn` is called with
    /// version 0. For [`Versioned::NotFound`], returns `None`.
    pub fn get_versioned_or_migrate<T: Serialize + DeserializeOwned>(
        key: impl AsRef<[u8]>,
        current_version: u32,
        migrate_fn: impl FnOnce(serde_json::Value, u32) -> Result<T, SysError>,
    ) -> Result<Option<T>, SysError> {
        let key = key.as_ref();

        match get_versioned::<T>(key, current_version)? {
            Versioned::Current(data) => Ok(Some(data)),
            Versioned::NeedsMigration {
                raw,
                stored_version,
            } => {
                let migrated = migrate_fn(raw, stored_version)?;
                set_versioned(key, &migrated, current_version)?;
                Ok(Some(migrated))
            },
            Versioned::Unversioned(raw) => {
                let migrated = migrate_fn(raw, 0)?;
                set_versioned(key, &migrated, current_version)?;
                Ok(Some(migrated))
            },
            Versioned::NotFound => Ok(None),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct TestData {
            name: String,
            count: u32,
        }

        // ---- Envelope serialization tests ----

        #[test]
        fn versioned_envelope_roundtrip() {
            let envelope = VersionedEnvelope {
                schema_version: 1,
                data: TestData {
                    name: "hello".into(),
                    count: 42,
                },
            };
            let json = serde_json::to_string(&envelope).unwrap();
            assert!(json.contains("\"__sv\":1"));
            assert!(json.contains("\"data\":{"));

            let parsed: VersionedEnvelope<TestData> = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.schema_version, 1);
            assert_eq!(
                parsed.data,
                TestData {
                    name: "hello".into(),
                    count: 42,
                }
            );
        }

        #[test]
        fn versioned_envelope_wire_format() {
            let envelope = VersionedEnvelope {
                schema_version: 3,
                data: serde_json::json!({"key": "value"}),
            };
            let json = serde_json::to_string(&envelope).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

            assert_eq!(parsed["__sv"], 3);
            assert_eq!(parsed["data"]["key"], "value");
        }

        // ---- parse_versioned logic tests ----

        #[test]
        fn parse_versioned_empty_bytes_returns_not_found() {
            let result = parse_versioned::<TestData>(b"", 1).unwrap();
            assert!(matches!(result, Versioned::NotFound));
        }

        #[test]
        fn parse_versioned_current_version_returns_current() {
            let bytes = br#"{"__sv":2,"data":{"name":"hello","count":42}}"#;
            let result = parse_versioned::<TestData>(bytes, 2).unwrap();
            match result {
                Versioned::Current(data) => {
                    assert_eq!(data.name, "hello");
                    assert_eq!(data.count, 42);
                },
                other => panic!("expected Current, got {other:?}"),
            }
        }

        #[test]
        fn parse_versioned_older_version_returns_needs_migration() {
            let bytes = br#"{"__sv":1,"data":{"name":"old","count":1}}"#;
            let result = parse_versioned::<TestData>(bytes, 3).unwrap();
            match result {
                Versioned::NeedsMigration {
                    raw,
                    stored_version,
                } => {
                    assert_eq!(stored_version, 1);
                    assert_eq!(raw["name"], "old");
                    assert_eq!(raw["count"], 1);
                },
                other => panic!("expected NeedsMigration, got {other:?}"),
            }
        }

        #[test]
        fn parse_versioned_newer_version_returns_error() {
            let bytes = br#"{"__sv":5,"data":{"name":"future","count":0}}"#;
            let result = parse_versioned::<TestData>(bytes, 2);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("newer than current"),
                "error should mention newer version: {err}"
            );
        }

        #[test]
        fn parse_versioned_plain_json_returns_unversioned() {
            let bytes = br#"{"name":"legacy","count":99}"#;
            let result = parse_versioned::<TestData>(bytes, 1).unwrap();
            match result {
                Versioned::Unversioned(val) => {
                    assert_eq!(val["name"], "legacy");
                    assert_eq!(val["count"], 99);
                },
                other => panic!("expected Unversioned, got {other:?}"),
            }
        }

        #[test]
        fn parse_versioned_malformed_sv_without_data_returns_error() {
            let bytes = br#"{"__sv":1,"payload":"something"}"#;
            let result = parse_versioned::<TestData>(bytes, 1);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("malformed"),
                "error should mention malformed envelope: {err}"
            );
        }

        #[test]
        fn parse_versioned_non_numeric_sv_returns_error() {
            let bytes = br#"{"__sv":"one","data":{}}"#;
            let result = parse_versioned::<TestData>(bytes, 1);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("malformed"),
                "error should mention malformed envelope: {err}"
            );
        }

        #[test]
        fn parse_versioned_version_zero_is_valid() {
            // Version 0 is a legitimate version (initial schema).
            let bytes = br#"{"__sv":0,"data":{"name":"v0","count":0}}"#;
            let result = parse_versioned::<TestData>(bytes, 0).unwrap();
            assert!(matches!(result, Versioned::Current(_)));
        }

        #[test]
        fn parse_versioned_invalid_json_returns_error() {
            let result = parse_versioned::<TestData>(b"not json", 1);
            assert!(result.is_err());
        }
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

pub mod types;

/// Capsule configuration (like `std::env`).
///
/// In the Astrid model, capsule config entries are the equivalent of
/// environment variables. The kernel injects them at load time.
pub mod env {
    use super::*;

    /// Well-known config key for the kernel's Unix domain socket path.
    pub const CONFIG_SOCKET_PATH: &str = "ASTRID_SOCKET_PATH";

    /// Read a config value as raw bytes. Like `std::env::var_os`.
    pub fn var_bytes(key: impl AsRef<[u8]>) -> Result<Vec<u8>, SysError> {
        let result = unsafe { astrid_get_config(key.as_ref().to_vec())? };
        Ok(result)
    }

    /// Read a config value as a UTF-8 string. Like `std::env::var`.
    pub fn var(key: impl AsRef<[u8]>) -> Result<String, SysError> {
        let bytes = var_bytes(key)?;
        String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))
    }
}

/// Wall-clock access (like `std::time`).
pub mod time {
    use super::*;

    /// Returns the current wall-clock time as milliseconds since the UNIX epoch.
    ///
    /// This is a host call - the WASM guest has no direct access to system time.
    /// Returns 0 if the host clock is unavailable.
    pub fn now_ms() -> Result<u64, SysError> {
        let bytes = unsafe { astrid_clock_ms()? };
        let s = String::from_utf8_lossy(&bytes);
        s.trim()
            .parse::<u64>()
            .map_err(|e| SysError::ApiError(format!("clock_ms parse error: {e}")))
    }
}

/// Structured logging.
pub mod log {
    use super::*;

    /// Log a message at the given level.
    pub fn log(level: impl AsRef<[u8]>, message: impl AsRef<[u8]>) -> Result<(), SysError> {
        unsafe { astrid_log(level.as_ref().to_vec(), message.as_ref().to_vec())? };
        Ok(())
    }

    /// Log at DEBUG level.
    pub fn debug(message: impl AsRef<[u8]>) -> Result<(), SysError> {
        log("debug", message)
    }

    /// Log at INFO level.
    pub fn info(message: impl AsRef<[u8]>) -> Result<(), SysError> {
        log("info", message)
    }

    /// Log at WARN level.
    pub fn warn(message: impl AsRef<[u8]>) -> Result<(), SysError> {
        log("warn", message)
    }

    /// Log at ERROR level.
    pub fn error(message: impl AsRef<[u8]>) -> Result<(), SysError> {
        log("error", message)
    }
}

/// OS runtime introspection and signaling.
pub mod runtime {
    use super::*;

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
    pub fn caller() -> Result<crate::types::CallerContext, SysError> {
        let bytes = unsafe { astrid_get_caller()? };
        serde_json::from_slice(&bytes)
            .map_err(|e| SysError::ApiError(format!("failed to parse caller context: {e}")))
    }

    /// Returns the kernel's Unix domain socket path.
    ///
    /// Reads from the well-known `ASTRID_SOCKET_PATH` config key that the
    /// kernel injects into every capsule at load time.
    pub fn socket_path() -> Result<String, SysError> {
        let raw = crate::env::var(crate::env::CONFIG_SOCKET_PATH)?;
        // var() returns JSON-encoded values (quoted strings).
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
        // Reject paths with null bytes - they would silently truncate at the OS level.
        if path.contains('\0') {
            return Err(SysError::ApiError(
                "ASTRID_SOCKET_PATH contains null byte".to_string(),
            ));
        }
        Ok(path)
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
        /// Return a subscription handle for use with `ipc::poll_bytes` / `ipc::recv_bytes`.
        #[must_use]
        pub fn subscription_handle(&self) -> ipc::SubscriptionHandle {
            ipc::SubscriptionHandle(self.handle_id.to_string().into_bytes())
        }

        /// Return the raw handle ID bytes (for lower-level interop).
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
        // SAFETY: FFI call to Extism host function. The host serializes
        // `HostState.interceptor_handles` to JSON and returns valid UTF-8 bytes.
        // Errors are propagated via the `?` operator.
        let bytes = unsafe { astrid_get_interceptor_handles()? };
        let bindings: Vec<InterceptorBinding> = serde_json::from_slice(&bytes)?;
        Ok(bindings)
    }

    /// Poll all interceptor subscriptions and dispatch pending events.
    ///
    /// For each binding with pending messages, calls
    /// `handler(action, envelope_bytes)` once with the full batch envelope
    /// (JSON with `messages` array, `dropped`, and `lagged` fields).
    /// Bindings with no pending messages are skipped.
    pub fn poll(
        bindings: &[InterceptorBinding],
        mut handler: impl FnMut(&str, &[u8]),
    ) -> Result<(), SysError> {
        #[derive(serde::Deserialize)]
        struct PollEnvelope {
            messages: Vec<serde_json::Value>,
        }

        for binding in bindings {
            let handle = binding.subscription_handle();
            let envelope = ipc::poll_bytes(&handle)?;

            // poll_bytes always returns a JSON envelope like
            // `{"messages":[],"dropped":0,"lagged":0}`. Check the
            // messages array before calling the handler.
            let parsed: PollEnvelope = serde_json::from_slice(&envelope)?;
            if !parsed.messages.is_empty() {
                handler(&binding.action, &envelope);
            }
        }
        Ok(())
    }
}

/// Request human approval for sensitive actions from within a capsule.
///
/// Any capsule can call [`approval::request`] to block until the frontend
/// user approves or denies an action. The host function checks the
/// `AllowanceStore` for a matching pattern first (instant path), and only
/// prompts the user when no allowance exists.
///
/// # Example
///
/// ```ignore
/// use astrid_sdk::prelude::*;
///
/// let result = approval::request("git push", "git push origin main", "high")?;
/// if !result.approved {
///     return Err(SysError::ApiError("Action denied by user".into()));
/// }
/// ```
/// Platform identity resolution and linking.
///
/// Capsules use this module to resolve platform-specific user identities
/// (e.g. Discord user IDs, Twitch usernames) to Astrid-native user IDs,
/// and to manage the links between them.
///
/// Requires the `identity` capability in `Capsule.toml`:
/// - `["resolve"]` - resolve platform users
/// - `["link"]` - resolve, link, unlink, and list links
/// - `["admin"]` - all of the above plus create new users
pub mod identity {
    use super::*;

    /// A resolved Astrid user returned by [`resolve`].
    #[derive(Debug)]
    pub struct ResolvedUser {
        /// The Astrid-native user ID (UUID).
        pub user_id: String,
        /// Optional display name.
        pub display_name: Option<String>,
    }

    /// A platform-to-Astrid identity link.
    #[derive(Debug)]
    pub struct Link {
        /// Platform name (e.g. "discord", "twitch").
        pub platform: String,
        /// Platform-specific user identifier.
        pub platform_user_id: String,
        /// The Astrid user this is linked to.
        pub astrid_user_id: String,
        /// When the link was created (RFC 3339).
        pub linked_at: String,
        /// How the link was established (e.g. "system", "chat_command").
        pub method: String,
    }

    /// Resolve a platform user to an Astrid user.
    ///
    /// Returns `Ok(Some(user))` if the platform identity is linked,
    /// `Ok(None)` if not found. Requires `identity = ["resolve"]` or higher.
    pub fn resolve(
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<ResolvedUser>, SysError> {
        #[derive(Serialize)]
        struct Req<'a> {
            platform: &'a str,
            platform_user_id: &'a str,
        }

        let req_bytes = serde_json::to_vec(&Req {
            platform,
            platform_user_id,
        })?;

        // SAFETY: FFI call to Extism host function.
        let resp_bytes = unsafe { astrid_identity_resolve(req_bytes)? };

        #[derive(Deserialize)]
        struct Resp {
            found: bool,
            user_id: Option<String>,
            display_name: Option<String>,
        }
        let resp: Resp = serde_json::from_slice(&resp_bytes)?;
        if resp.found {
            Ok(Some(ResolvedUser {
                user_id: resp.user_id.unwrap_or_default(),
                display_name: resp.display_name,
            }))
        } else {
            Ok(None)
        }
    }

    /// Link a platform identity to an Astrid user.
    ///
    /// - `method` describes how the link was established (e.g. "chat_command", "system").
    ///
    /// Returns the created link on success. Requires `identity = ["link"]` or higher.
    pub fn link(
        platform: &str,
        platform_user_id: &str,
        astrid_user_id: &str,
        method: &str,
    ) -> Result<Link, SysError> {
        #[derive(Serialize)]
        struct Req<'a> {
            platform: &'a str,
            platform_user_id: &'a str,
            astrid_user_id: &'a str,
            method: &'a str,
        }

        let req_bytes = serde_json::to_vec(&Req {
            platform,
            platform_user_id,
            astrid_user_id,
            method,
        })?;

        // SAFETY: FFI call to Extism host function.
        let resp_bytes = unsafe { astrid_identity_link(req_bytes)? };

        #[derive(Deserialize)]
        struct LinkInfo {
            platform: String,
            platform_user_id: String,
            astrid_user_id: String,
            linked_at: String,
            method: String,
        }
        #[derive(Deserialize)]
        struct Resp {
            ok: bool,
            error: Option<String>,
            link: Option<LinkInfo>,
        }
        let resp: Resp = serde_json::from_slice(&resp_bytes)?;
        if !resp.ok {
            return Err(SysError::ApiError(
                resp.error.unwrap_or_else(|| "identity link failed".into()),
            ));
        }
        let l = resp
            .link
            .ok_or_else(|| SysError::ApiError("missing link in response".into()))?;
        Ok(Link {
            platform: l.platform,
            platform_user_id: l.platform_user_id,
            astrid_user_id: l.astrid_user_id,
            linked_at: l.linked_at,
            method: l.method,
        })
    }

    /// Unlink a platform identity from its Astrid user.
    ///
    /// Returns `true` if a link was removed, `false` if none existed.
    /// Requires `identity = ["link"]` or higher.
    pub fn unlink(platform: &str, platform_user_id: &str) -> Result<bool, SysError> {
        #[derive(Serialize)]
        struct Req<'a> {
            platform: &'a str,
            platform_user_id: &'a str,
        }

        let req_bytes = serde_json::to_vec(&Req {
            platform,
            platform_user_id,
        })?;

        // SAFETY: FFI call to Extism host function.
        let resp_bytes = unsafe { astrid_identity_unlink(req_bytes)? };

        #[derive(Deserialize)]
        struct Resp {
            ok: bool,
            error: Option<String>,
            removed: Option<bool>,
        }
        let resp: Resp = serde_json::from_slice(&resp_bytes)?;
        if !resp.ok {
            return Err(SysError::ApiError(
                resp.error
                    .unwrap_or_else(|| "identity unlink failed".into()),
            ));
        }
        Ok(resp.removed.unwrap_or(false))
    }

    /// Create a new Astrid user.
    ///
    /// Returns the UUID of the newly created user.
    /// Requires `identity = ["admin"]`.
    pub fn create_user(display_name: Option<&str>) -> Result<String, SysError> {
        #[derive(Serialize)]
        struct Req<'a> {
            display_name: Option<&'a str>,
        }

        let req_bytes = serde_json::to_vec(&Req { display_name })?;

        // SAFETY: FFI call to Extism host function.
        let resp_bytes = unsafe { astrid_identity_create_user(req_bytes)? };

        #[derive(Deserialize)]
        struct Resp {
            ok: bool,
            error: Option<String>,
            user_id: Option<String>,
        }
        let resp: Resp = serde_json::from_slice(&resp_bytes)?;
        if !resp.ok {
            return Err(SysError::ApiError(
                resp.error
                    .unwrap_or_else(|| "identity create_user failed".into()),
            ));
        }
        resp.user_id
            .ok_or_else(|| SysError::ApiError("missing user_id in response".into()))
    }

    /// List all platform links for an Astrid user.
    ///
    /// Returns all linked platform identities for the given user UUID.
    /// Requires `identity = ["link"]` or higher.
    pub fn list_links(astrid_user_id: &str) -> Result<Vec<Link>, SysError> {
        #[derive(Serialize)]
        struct Req<'a> {
            astrid_user_id: &'a str,
        }

        let req_bytes = serde_json::to_vec(&Req { astrid_user_id })?;

        // SAFETY: FFI call to Extism host function.
        let resp_bytes = unsafe { astrid_identity_list_links(req_bytes)? };

        #[derive(Deserialize)]
        struct LinkInfo {
            platform: String,
            platform_user_id: String,
            astrid_user_id: String,
            linked_at: String,
            method: String,
        }
        #[derive(Deserialize)]
        struct Resp {
            ok: bool,
            error: Option<String>,
            links: Option<Vec<LinkInfo>>,
        }
        let resp: Resp = serde_json::from_slice(&resp_bytes)?;
        if !resp.ok {
            return Err(SysError::ApiError(
                resp.error
                    .unwrap_or_else(|| "identity list_links failed".into()),
            ));
        }
        Ok(resp
            .links
            .unwrap_or_default()
            .into_iter()
            .map(|l| Link {
                platform: l.platform,
                platform_user_id: l.platform_user_id,
                astrid_user_id: l.astrid_user_id,
                linked_at: l.linked_at,
                method: l.method,
            })
            .collect())
    }
}

pub mod approval {
    use super::*;

    /// The result of an approval request.
    #[derive(Debug)]
    pub struct ApprovalResult {
        /// Whether the action was approved.
        pub approved: bool,
        /// The decision string: "approve", "approve_session",
        /// "approve_always", "deny", or "allowance" (auto-approved).
        pub decision: String,
    }

    /// Request human approval for a sensitive action.
    ///
    /// Blocks the capsule until the frontend user responds or the request
    /// times out. If an existing allowance matches, returns immediately
    /// without prompting.
    ///
    /// - `action` - short description of the action (e.g. "git push")
    /// - `resource` - full resource identifier (e.g. "git push origin main")
    /// - `risk_level` - one of "low", "medium", "high", "critical"
    pub fn request(
        action: &str,
        resource: &str,
        risk_level: &str,
    ) -> Result<ApprovalResult, SysError> {
        #[derive(Serialize)]
        struct ApprovalRequest<'a> {
            action: &'a str,
            resource: &'a str,
            risk_level: &'a str,
        }

        let req = ApprovalRequest {
            action,
            resource,
            risk_level,
        };
        let req_bytes = serde_json::to_vec(&req)?;

        // SAFETY: FFI call to Extism host function. The host checks the
        // AllowanceStore, publishes ApprovalRequired if needed, blocks
        // until a response arrives, and returns a JSON result.
        let resp_bytes = unsafe { astrid_request_approval(req_bytes)? };

        #[derive(Deserialize)]
        struct ApprovalResp {
            approved: bool,
            decision: String,
        }
        let resp: ApprovalResp = serde_json::from_slice(&resp_bytes)?;
        Ok(ApprovalResult {
            approved: resp.approved,
            decision: resp.decision,
        })
    }
}

pub mod prelude {
    pub use crate::{
        SysError,
        // Astrid-specific modules
        approval,
        cron,
        elicit,
        // std-mirrored modules
        env,
        fs,
        hooks,
        http,
        identity,
        interceptors,
        ipc,
        kv,
        log,
        net,
        process,
        runtime,
        time,
        uplink,
    };

    #[cfg(feature = "derive")]
    pub use astrid_sdk_macros::capsule;
}
