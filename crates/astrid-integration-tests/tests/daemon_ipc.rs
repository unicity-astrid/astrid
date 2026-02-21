//! Integration tests for daemon `WebSocket` IPC.
//!
//! These tests verify basic connectivity between the daemon server and
//! client over JSON-RPC 2.0 `WebSocket`. They are deferred because the
//! daemon currently uses `ClaudeProvider` directly rather than a generic
//! `LlmProvider`, so injecting a mock LLM requires refactoring.
//!
//! For now, we test the RPC traits and types exist and compile.

// The daemon server (`DaemonServer::start()`) requires:
// 1. A real `ClaudeProvider` (hardcoded, not generic over LlmProvider)
// 2. File system state (PID file, port file)
// 3. MCP servers config
//
// This means we cannot easily inject a MockLlmProvider into the daemon.
// Full daemon IPC tests are deferred until the gateway crate is refactored
// to accept a generic LlmProvider.

#[test]
fn test_daemon_types_compile() {
    // Verify that the key daemon types exist and are accessible
    use astrid_mcp::ServersConfig;
    let _config = ServersConfig::default();

    // SessionId can be created
    use astrid_core::SessionId;
    let _id = SessionId::new();
}
