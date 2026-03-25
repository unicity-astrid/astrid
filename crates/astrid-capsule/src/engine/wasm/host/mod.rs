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

// Host registration is handled by `Capsule::add_to_linker()` which wires
// all Host trait impls on `HostState` into the wasmtime `Linker`.

// The `types` interface only defines shared WIT types (records, enums) — no
// functions. The generated Host trait is empty but must still be implemented
// for `Capsule::add_to_linker` to accept HostState.
impl crate::engine::wasm::bindings::astrid::capsule::types::Host
    for crate::engine::wasm::host_state::HostState
{
}

// --- Extism compatibility stub ---
// The WasmHandler in astrid-hooks still uses Extism (commit 6 migrates it).
// This no-op stub keeps that crate compiling until it is migrated.

/// Stub: registers no host functions. The real registration happens via
/// `Capsule::add_to_linker()` in the wasmtime Component Model path.
#[deprecated(note = "Extism is being replaced by wasmtime Component Model")]
pub fn register_host_functions(
    builder: extism::PluginBuilder,
    _user_data: extism::UserData<crate::engine::wasm::host_state::HostState>,
) -> extism::PluginBuilder {
    builder
}
