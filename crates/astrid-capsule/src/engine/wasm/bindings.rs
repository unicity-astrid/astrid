//! Component Model bindings generated from `wit/astrid-capsule.wit`.
//!
//! This module uses `wasmtime::component::bindgen!` to generate:
//! - Host trait types for each WIT interface (e.g. `fs::Host`, `ipc::Host`)
//! - Guest export bindings (the `Capsule` struct with `call_*` methods)
//! - Typed Rust structs for all WIT record types

wasmtime::component::bindgen!({
    world: "capsule",
    path: "../../wit",
});
