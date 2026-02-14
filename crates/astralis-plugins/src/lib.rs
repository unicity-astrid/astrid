//! Plugin trait and registry for the Astralis secure agent runtime SDK.
//!
//! Provides the core abstractions for extending Astralis with plugins:
//!
//! - [`PluginId`]: Stable, human-readable plugin identifier
//! - [`PluginManifest`]: Describes a plugin's identity, entry point, and capabilities
//! - [`Plugin`]: Trait for plugin lifecycle (load/unload) and tool provision
//! - [`PluginTool`]: Trait for tools provided by plugins (mirrors `BuiltinTool`)
//! - [`PluginContext`] / [`PluginToolContext`]: Execution contexts with scoped KV storage
//! - [`PluginRegistry`]: Registry for loaded plugins with cross-plugin tool lookup
//! - [`discover_manifests`]: Filesystem discovery of `plugin.toml` manifests
//!
//! # Tool Naming Convention
//!
//! Plugin tools are exposed to the LLM as `plugin:{plugin_id}:{tool_name}`,
//! which avoids collision with built-in tools (no colons) and MCP tools
//! (`server:tool` — single colon).
//!
//! # Storage Isolation
//!
//! Each plugin gets a [`ScopedKvStore`](astralis_storage::ScopedKvStore) pre-bound
//! to the namespace `plugin:{plugin_id}`. Plugins cannot access each other's data.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod context;
pub mod discovery;
pub mod error;
pub mod manifest;
pub mod mcp_plugin;
pub mod plugin;
pub mod registry;
pub mod sandbox;
pub mod security;
pub mod tool;
pub mod wasm;

pub use context::{PluginContext, PluginToolContext};
pub use discovery::{discover_manifests, load_manifest, load_manifests_from_dir};
pub use error::{PluginError, PluginResult};
pub use manifest::{PluginCapability, PluginEntryPoint, PluginManifest};
pub use mcp_plugin::{McpPlugin, create_plugin};
pub use plugin::{Plugin, PluginId, PluginState};
pub use registry::{PluginRegistry, PluginToolDefinition};
pub use sandbox::SandboxProfile;
pub use security::PluginSecurityGate;
pub use tool::PluginTool;
pub use wasm::{WasmPlugin, WasmPluginLoader, WasmPluginTool};
