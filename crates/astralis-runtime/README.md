# astralis-runtime

Agent orchestration and session management for Astralis.

## Overview

This crate provides the core runtime that coordinates all Astralis components:

- **LLM Provider** - Language model interactions
- **MCP Client** - Tool execution via Model Context Protocol
- **Capability Store** - Authorization and access control
- **Audit Log** - Security logging with cryptographic proofs

## Features

- **Agent Runtime** - Unified orchestration of LLM, MCP, and security layers
- **Session Management** - Persistent sessions with metadata tracking
- **Context Management** - Auto-summarization to stay within token limits
- **Streaming Support** - Real-time response streaming via Frontend trait

## Usage

```rust
use astralis_runtime::{AgentRuntime, RuntimeConfig, SessionStore};
use astralis_llm::ClaudeProvider;
use astralis_mcp::McpClient;
use astralis_audit::AuditLog;
use astralis_crypto::KeyPair;

// Create components
let llm = ClaudeProvider::from_env()?;
let mcp = McpClient::from_default_config()?;
let audit_key = KeyPair::generate();
let runtime_key = KeyPair::generate();
let audit = AuditLog::in_memory(audit_key)?;
let sessions = SessionStore::default_dir()?;

// Create runtime
let runtime = AgentRuntime::new(
    llm,
    mcp,
    audit,
    sessions,
    runtime_key,
    RuntimeConfig::default(),
);

// Create a session
let mut session = runtime.create_session(None);

// Run a turn (requires a Frontend implementation)
// runtime.run_turn_streaming(&mut session, "Hello!", &frontend).await?;
```

## Key Types

| Type | Description |
|------|-------------|
| `AgentRuntime` | Main orchestrator coordinating all components |
| `RuntimeConfig` | Configuration for runtime behavior |
| `AgentSession` | Active session with conversation state |
| `SessionStore` | Persistent storage for sessions |
| `ContextManager` | Manages context window and summarization |

## License

This crate is licensed under the MIT license.
