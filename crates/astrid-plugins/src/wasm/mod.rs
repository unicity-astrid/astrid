//! WASM plugin runtime powered by Extism.
//!
//! This module provides the concrete [`WasmPlugin`] implementation that loads
//! `.wasm` files via Extism, registers host functions matching the WIT `host`
//! interface, and routes tool calls through the WASM guest.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────┐     ┌───────────────┐     ┌──────────────┐
//! │ WasmPlugin   │────▶│ extism::Plugin │────▶│ WASM Guest   │
//! │ (Plugin trait)│     │ (Arc<Mutex>)  │     │ execute-tool  │
//! └──────────────┘     └───────────────┘     │ describe-tools│
//!                            │                └──────────────┘
//!                            ▼
//!                      ┌──────────────┐
//!                      │ Host Fns (7) │
//!                      │ via UserData │
//!                      │ <HostState>  │
//!                      └──────────────┘
//! ```
//!
//! # Async Bridging
//!
//! Extism host functions are synchronous. Async operations (KV store, security
//! checks, HTTP) are bridged via `tokio::task::block_in_place` +
//! `Handle::block_on()`. This **requires the multi-threaded tokio runtime**.
//!
//! All tests must use `#[tokio::test(flavor = "multi_thread")]`.

pub mod host;
pub mod host_state;
pub mod loader;
pub mod plugin;
pub mod tool;

pub use loader::WasmPluginLoader;
pub use plugin::WasmPlugin;
pub use tool::WasmPluginTool;
