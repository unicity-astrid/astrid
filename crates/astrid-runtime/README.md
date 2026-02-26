# astrid-runtime

[![Crates.io](https://img.shields.io/crates/v/astrid-runtime)](https://crates.io/crates/astrid-runtime)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The execution engine of the Astralis OS.

`astrid-runtime` serves as the brainstem of Astralis. It coordinates the core agentic loop, binding together language model interactions, Model Context Protocol (MCP) tool execution, and the overarching security layers. By providing robust session state management, context summarization, and a powerful hierarchical sub-agent orchestration system, this crate transforms raw model capabilities into persistent, safely executing autonomous agents.

## Core Features

- **Unified Agentic Loop**: Orchestrates `astrid-llm`, `astrid-mcp`, and `astrid-capabilities` into a cohesive execution cycle, managing prompt generation, tool interception, and recursive task resolution.
- **Hierarchical Sub-Agent Orchestration**: Safely spawn and manage concurrent sub-agents. The runtime enforces strict depth limits and concurrency constraints while providing cooperative cancellation for entire execution subtrees.
- **Persistent Session State**: Full serialization of conversation history, security allowances, budget snapshots, and workspace boundaries. Sessions can be cleanly serialized to disk and resumed across daemon restarts without losing critical security context.
- **Inherited Security and Budgeting**: Child sessions automatically inherit the parent's `CapabilityStore` and `BudgetTracker`. This ensures that project-level permissions and financial limits remain globally enforced, preventing nested agents from bypassing overarching constraints.
- **Intelligent Context Management**: Proactive token tracking and auto-summarization to prevent context window exhaustion during long-running tasks or extensive sub-agent operations.
- **Cryptographic Audit Integration**: Seamlessly links with `astrid-audit` to provably log session lifecycles, sub-agent spawns, and authorization boundaries.

## Architecture

The runtime revolves around the `AgentRuntime` struct, which holds all shared resources including LLM providers, MCP clients, tool registries, and the sub-agent pool. 

When a user or a system event initiates a task, the runtime provisions an `AgentSession`. During the agentic loop, `astrid-runtime` evaluates the current context and issues requests to the LLM. When the model requests a tool execution, the runtime intercepts the call, routing it through a `SecurityInterceptor`. This interceptor verifies permissions against the session's capability store and checks available budgets before delegating execution to the appropriate MCP client or internal plugin.

### Sub-Agent Lifecycle

A key responsibility of this crate is managing concurrent, autonomous execution. The `SubAgentPool` provides a semaphore-backed environment for spawning child agents:

1. **Spawning**: When a parent agent requests a sub-agent, the `SubAgentExecutor` acquires a concurrency permit and increments the depth counter.
2. **Inheritance**: The child agent receives a fresh `ApprovalManager` for independent request handling, but shares the `Arc` pointers for the parent's allowance store and budget tracker.
3. **Execution**: The sub-agent runs its own isolated agentic loop, subject to timeout configurations and cooperative cancellation tokens.
4. **Resolution**: Upon completion or failure, the result is returned to the parent agent, the permit is released, and the execution trace is archived in the pool's history buffer.

## Quick Start

Initialize the runtime and dispatch a session:

```rust
use astrid_runtime::{AgentRuntime, RuntimeConfig, SessionStore};
use astrid_llm::{ClaudeProvider, ProviderConfig};
use astrid_mcp::McpClient;
use astrid_audit::AuditLog;
use astrid_crypto::KeyPair;

async fn initialize_runtime() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Provision underlying dependencies
    let llm = ClaudeProvider::new(ProviderConfig::new("api-key", "claude-sonnet-4-20250514"));
    let mcp = McpClient::from_default_config()?;
    
    let audit_key = KeyPair::generate();
    let runtime_key = KeyPair::generate();
    let audit = AuditLog::in_memory(audit_key);
    
    let home = astrid_core::dirs::AstridHome::resolve()?;
    let sessions = SessionStore::from_home(&home);

    // 2. Construct the orchestrator
    // Note: Wrapping in Arc and calling `new_arc` is required 
    // to enable the sub-agent spawner injection.
    let runtime = AgentRuntime::new_arc(
        llm,
        mcp,
        audit,
        sessions,
        runtime_key,
        RuntimeConfig::default(),
        None,
        None,
    );

    // 3. Instantiate a persistent session
    let mut session = runtime.create_session(None);

    // The runtime is now ready to execute turns via `run_turn_streaming`
    
    Ok(())
}
```

## API Reference

### Core Types

- `AgentRuntime`: The primary orchestrator. Coordinates shared state and manages the execution loops.
- `AgentSession`: Tracks conversation state, capabilities, metadata, and accumulated budget spend for a specific execution thread.
- `SubAgentPool`: Enforces concurrency and depth limits, managing lifecycle and metrics for asynchronous child agents.
- `SubAgentExecutor`: Implements the `SubAgentSpawner` trait, injecting the agentic loop execution capability directly into the tool context.
- `ContextManager`: Monitors token usage across the session and orchestrates context summarization to preserve model reasoning capabilities.

## Development

```bash
cargo test -p astrid-runtime
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
