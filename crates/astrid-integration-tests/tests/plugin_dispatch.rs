//! Integration tests for plugin tool dispatch in the agent runtime.
//!
//! Verifies that plugin tools appear in the LLM tool list, route through
//! `execute_plugin_tool` (not the MCP path), and fire security interceptor
//! and hooks correctly.
//!
//! Tests are organized into focused submodules:
//! - `fixtures`      — shared test plugin types (`TestPlugin`, `EchoTool`, etc.)
//! - `helpers`       — runtime builder helpers
//! - `dispatch_tests`     — core dispatch, not-found, and registry builder tests
//! - `security_tests`     — security interceptor denial and workspace boundary
//! - `error_multi_tests`  — error propagation and multi-plugin dispatch
//! - `hook_tests`         — `PreToolCall` blocking, `PostToolCall`, and `ToolError` hooks
//! - `hot_reload_tests`   — plugin unloaded/failed mid-turn race conditions
//! - `audit_tests`        — audit log entry verification
//! - `tool_name_tests`    — special characters and colon-containing tool names
//! - `kv_store_tests`     — KV store session isolation and cleanup

mod common;

#[path = "plugin_dispatch/fixtures.rs"]
mod fixtures;

#[path = "plugin_dispatch/helpers.rs"]
mod helpers;

#[path = "plugin_dispatch/dispatch_tests.rs"]
mod dispatch_tests;

#[path = "plugin_dispatch/security_tests.rs"]
mod security_tests;

#[path = "plugin_dispatch/error_multi_tests.rs"]
mod error_multi_tests;

#[path = "plugin_dispatch/hook_tests.rs"]
mod hook_tests;

#[path = "plugin_dispatch/hot_reload_tests.rs"]
mod hot_reload_tests;

#[path = "plugin_dispatch/audit_tests.rs"]
mod audit_tests;

#[path = "plugin_dispatch/tool_name_tests.rs"]
mod tool_name_tests;

#[path = "plugin_dispatch/kv_store_tests.rs"]
mod kv_store_tests;
