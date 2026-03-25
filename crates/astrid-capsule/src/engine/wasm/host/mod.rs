//! Host function implementations for the wasmtime Component Model.
//!
//! Each submodule implements the corresponding `Host` trait from the
//! WIT-generated bindings on `HostState`. The trait implementations
//! are automatically wired to the wasmtime linker via
//! `Capsule::add_to_linker()`.

/// Capsule-level approval requests.
pub(crate) mod approval;
/// Elicit lifecycle API (install/upgrade user input collection).
pub(crate) mod elicit;
/// File system operations for plugins.
pub(crate) mod fs;
/// HTTP network executions for plugins.
pub mod http;
/// Identity operations (resolve, link, create user).
pub(crate) mod identity;
/// Inter-Process Communication bus.
pub(crate) mod ipc;
/// Key-Value persistent storage primitives.
pub(crate) mod kv;
pub(crate) mod net;
/// Process spawning and sandboxing.
pub mod process;
/// System configuration primitives.
pub mod sys;
/// Uplink communications with host capabilities.
pub(crate) mod uplink;
/// Utility functions for WASM host implementations.
pub(crate) mod util;

// --- Extism compatibility stub ---
// The old Extism dispatch code (WasmEngine, astrid-hooks WasmHandler) still calls
// this function. It's a no-op stub until the engine is rewritten (commit 5) and
// the hooks handler is migrated (commit 6).

/// Stub: registers no host functions. The real registration happens via
/// `Capsule::add_to_linker()` in the wasmtime Component Model path.
pub fn register_host_functions(
    builder: extism::PluginBuilder,
    _user_data: extism::UserData<crate::engine::wasm::host_state::HostState>,
) -> extism::PluginBuilder {
    builder
}
