//! Raw FFI bindings for the Astrid OS System API (The Airlocks).
//!
//! This crate defines the absolute lowest-level, mathematically pure ABI.
//! Every single parameter and return type across the WASM boundary is
//! represented as raw bytes (`Vec<u8>`).
//!
//! This provides true OS-level primitiveness: file paths can contain non-UTF-8
//! sequences, IPC topics can be binary hashes, and the Kernel never wastes CPU
//! validating string encodings. All ergonomic serialization is handled entirely
//! by the `astrid-sdk` User-Space layer.

#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    // -----------------------------------------------------------------------
    // File System (VFS) Operations
    // -----------------------------------------------------------------------
    /// Check if a VFS path exists.
    pub fn astrid_fs_exists(path: Vec<u8>) -> Vec<u8>;
    /// Create a directory in the VFS.
    pub fn astrid_fs_mkdir(path: Vec<u8>);
    /// Read a directory in the VFS.
    pub fn astrid_fs_readdir(path: Vec<u8>) -> Vec<u8>;
    /// Get stats for a VFS path.
    pub fn astrid_fs_stat(path: Vec<u8>) -> Vec<u8>;
    /// Delete a file or directory in the VFS.
    pub fn astrid_fs_unlink(path: Vec<u8>);

    /// Read a file's contents from the VFS.
    pub fn astrid_read_file(path: Vec<u8>) -> Vec<u8>;
    /// Write contents to a file in the VFS.
    pub fn astrid_write_file(path: Vec<u8>, content: Vec<u8>);

    // -----------------------------------------------------------------------
    // Inter-Process Communication (Message Bus & Uplinks)
    // -----------------------------------------------------------------------
    /// Publish a message to the OS event bus.
    pub fn astrid_ipc_publish(topic: Vec<u8>, payload: Vec<u8>);
    /// Subscribe to a topic on the OS event bus.
    pub fn astrid_ipc_subscribe(topic: Vec<u8>) -> Vec<u8>;
    /// Unsubscribe from the OS event bus.
    pub fn astrid_ipc_unsubscribe(handle: Vec<u8>);
    /// Poll for the next message on an IPC subscription handle.
    pub fn astrid_ipc_poll(handle: Vec<u8>) -> Vec<u8>;

    /// Register a direct uplink (frontend).
    pub fn astrid_uplink_register(name: Vec<u8>, platform: Vec<u8>, profile: Vec<u8>) -> Vec<u8>;
    /// Send a message via a direct uplink.
    pub fn astrid_uplink_send(
        uplink_id: Vec<u8>,
        platform_user_id: Vec<u8>,
        content: Vec<u8>,
    ) -> Vec<u8>;

    // -----------------------------------------------------------------------
    // Storage & Configuration
    // -----------------------------------------------------------------------
    /// Get a value from the KV store.
    pub fn astrid_kv_get(key: Vec<u8>) -> Vec<u8>;
    /// Set a value in the KV store.
    pub fn astrid_kv_set(key: Vec<u8>, value: Vec<u8>);

    /// Get a system configuration string.
    pub fn astrid_get_config(key: Vec<u8>) -> Vec<u8>;

    // -----------------------------------------------------------------------
    // General System (Network, Logging, & Scheduling)
    // -----------------------------------------------------------------------
    /// Issue an HTTP request.
    pub fn astrid_http_request(request_bytes: Vec<u8>) -> Vec<u8>;
    /// Log a message to the OS journal.
    pub fn astrid_log(level: Vec<u8>, message: Vec<u8>);
    /// Schedule a dynamic cron job to trigger the capsule later.
    pub fn astrid_cron_schedule(name: Vec<u8>, schedule: Vec<u8>, payload: Vec<u8>);
    /// Cancel a dynamic cron job.
    pub fn astrid_cron_cancel(name: Vec<u8>);

    // -----------------------------------------------------------------------
    // Host Execution (The Escape Hatch)
    // -----------------------------------------------------------------------
    /// Spawn a native host process. Requires the `host_process` capability.
    pub fn astrid_spawn_host(cmd_and_args_json: Vec<u8>) -> Vec<u8>;
}
