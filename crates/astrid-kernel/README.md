# astrid-kernel

[![Crates.io](https://img.shields.io/crates/v/astrid-kernel)](https://crates.io/crates/astrid-kernel)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The daemon layer for the Astrid secure agent runtime, managing multi-agent lifecycles, JSON-RPC communication, and comprehensive system health.

`astrid-kernel` serves as the central nervous system of the Astralis OS. While `astrid-runtime` handles the intricate mechanics of context windows and tool execution, the gateway acts as the long-running daemon that binds these mechanics to the outside world. It abstracts the complexity of MCP server management, plugin lifecycles, and agent orchestration behind a clean, event-driven JSON-RPC over WebSocket interface.

Whether you are building a CLI, a web frontend, or a custom desktop client, `astrid-kernel` provides the stable, observable boundary that ensures your agents run continuously, securely, and predictably.

## Core Features

* **JSON-RPC over WebSocket**: Provides a bi-directional communication channel using `jsonrpsee`, enabling real-time streaming of LLM tokens, tool execution states, and human-in-the-loop approval requests.
* **Multi-Agent Management**: Spawns, pauses, and terminates multiple agent instances concurrently. Manages sub-agent pooling to optimize resource usage across complex, multi-turn tasks.
* **Hot-Reloading Configuration**: Monitors configuration files (like `gateway.toml`) via the `notify` crate, applying changes to models, routing, and environment settings dynamically without requiring a daemon restart or dropping active sessions.
* **Comprehensive Health Diagnostics**: Aggregates the operational status of MCP servers, agent pools, audit logs, and approval queues into a unified health state (Healthy, Degraded, Unhealthy).
* **State Persistence**: Manages session checkpointing and ensures graceful shutdowns, preventing data loss during unexpected terminations.

## Architecture

The gateway operates strictly as a facade over the underlying orchestration layers. It adheres to the "Island Principle" of the Astralis architecture, keeping routing, health, and server mechanics tightly encapsulated.

```text
Client Application (CLI, Web, GUI)
               │
      [JSON-RPC / WebSocket]
               │
       astrid-kernel (Daemon)
       ├── Configuration & Hot-Reload
       ├── Multi-Agent Manager
       ├── Message Router
       └── Health Diagnostics
               │
       astrid-runtime (Orchestrator)
       ├── Session & Context Management
       ├── astrid-llm (Provider Layer)
       └── astrid-mcp (Tool Layer)
```

### Internal Module Structure

* `server/`: Handles the WebSocket connection lifecycle, incoming routing, and daemon startup sequences.
* `rpc.rs`: Defines the `jsonrpsee` server trait, wire types (`ToolInfo`, `DaemonStatus`, `SessionInfo`), and event serialization logic.
* `manager.rs`: Implements `AgentManager` and `AgentHandle` for tracking state, uptime, and request counts across concurrent agents.
* `health.rs`: Provides the `HealthStatus` aggregator, evaluating system thresholds (e.g., pending approval queues, MCP bridge connectivity, audit availability).
* `router.rs`: Manages `ChannelBinding` and message propagation between the frontend clients and the internal runtime channels.

## Quick Start

The gateway is designed to be embedded as the primary execution loop of a host process.

```rust
use astrid_kernel::{GatewayConfig, GatewayRuntime};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration, supporting hot-reload watch streams natively
    let config = GatewayConfig::load("~/.astrid/gateway.toml")?;
    
    // Initialize the gateway daemon
    let runtime = GatewayRuntime::new(config)?;

    // Block the main thread and process incoming WebSocket connections
    // and internal agent lifecycles until a shutdown signal is received.
    runtime.run().await?;

    Ok(())
}
```

### Core RPC Methods
* `createSession` / `resumeSession`: Initializes or restores a workspace-bound context.
* `sendInput`: Dispatches user input to the active agent.
* `approvalResponse` / `elicitationResponse`: Returns human-in-the-loop decisions back to the daemon.
* `subscribeEvents`: Opens a streaming subscription for real-time runtime events.

### Streaming Events (`DaemonEvent`)
Clients consuming the WebSocket stream receive real-time, granular telemetry:
* `Text`: Streaming LLM tokens.
* `ToolCallStart` / `ToolCallResult`: Telemetry for MCP and internal tool execution.
* `ApprovalNeeded`: Halts execution and waits for client authorization based on security policies.
* `Usage`: Token burn rate and context window tracking.
* `CapsuleLoaded` / `CapsuleFailed`: Plugin lifecycle updates.

## Development

When modifying the gateway, ensure you respect the closed-set enum architecture for wire types and events. Any new variant added to `DaemonEvent` must be comprehensively tested for Serde round-tripping, as it directly impacts all external frontends.

To run the test suite for this crate:

```bash
cargo test -p astrid-kernel -- --quiet
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
