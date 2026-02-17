//! Plugin trait and registry for the Astrid secure agent runtime.
//!
//! Provides the core abstractions for extending Astrid with plugins:
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
//! Each plugin gets a [`ScopedKvStore`](astrid_storage::ScopedKvStore) pre-bound
//! to the namespace `plugin:{plugin_id}`. Plugins cannot access each other's data.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod context;
pub mod discovery;
pub mod error;
#[cfg(feature = "http")]
pub mod git_install;
pub mod lockfile;
pub mod manifest;
pub mod mcp_plugin;
#[cfg(feature = "http")]
pub mod npm;
pub mod plugin;
pub mod plugin_dirs;
pub mod registry;
pub mod sandbox;
pub mod security;
pub mod tool;
pub mod wasm;
#[cfg(feature = "watch")]
pub mod watcher;

pub use context::{PluginContext, PluginToolContext};
pub use discovery::{discover_manifests, load_manifest, load_manifests_from_dir};
pub use error::{PluginError, PluginResult};
#[cfg(feature = "http")]
pub use git_install::GitSource;
pub use lockfile::{IntegrityViolation, LockedPlugin, PluginLockfile, PluginSource};
pub use manifest::{ManifestConnector, PluginCapability, PluginEntryPoint, PluginManifest};
pub use mcp_plugin::{McpPlugin, create_plugin};
pub use plugin::{Plugin, PluginId, PluginState};
pub use registry::{PluginRegistry, PluginToolDefinition};
pub use sandbox::SandboxProfile;
pub use security::PluginSecurityGate;
pub use tool::PluginTool;
pub use wasm::{WasmPlugin, WasmPluginLoader, WasmPluginTool};
