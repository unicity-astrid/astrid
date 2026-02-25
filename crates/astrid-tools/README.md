# astrid-tools

[![Crates.io](https://img.shields.io/crates/v/astrid-tools)](https://crates.io/crates/astrid-tools)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The high-speed, in-process tool execution engine for the Astralis agent runtime.

When an AI agent needs to interact with the world, it uses tools. While the legacy Model Context Protocol (MCP) requires spinning up child processes and communicating over standard I/O, `astrid-tools` implements the 9 most critical, high-frequency operations directly in-memory as native Rust function calls.

This architecture eliminates IPC overhead, prevents context poisoning, and ensures that hot-path operations—like navigating directories, reading files, and running shell commands—execute with near-zero latency. As a core component of Astralis, this crate relies on the overarching security interceptors to handle capability enforcement, focusing entirely on execution speed and deterministic behavior.

## Core Features

- **In-Process Execution:** Bypasses external process spawning for core filesystem and execution tasks, drastically reducing turn latency.
- **Persistent State:** A shared `ToolContext` maintains the working directory across discrete bash invocations and propagates sub-agent capabilities.
- **LLM-Optimized Output:** Automatically truncates massive outputs (preventing context window blowouts) and formats read operations with line numbers to assist in precise patching.
- **Cryptographic Sentinels:** Uses unguessable UUIDs to parse bash output states, making it impossible for untrusted command output to spoof the working directory.

## Architecture

Every tool implements the asynchronous `BuiltinTool` trait:

```rust
#[async_trait::async_trait]
pub trait BuiltinTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult;
}
```

### The Tool Arsenal

The crate provides 9 built-in tools tailored for agentic workflows:

- `bash`: Executes shell commands while maintaining a persistent working directory. Uses secure UUID sentinels to capture exit codes and state changes reliably.
- `read_file`: Retrieves file contents formatted with line numbers, critical for accurate file editing. Enforces a 30,000 character limit to protect context windows.
- `write_file`: Atomically writes content to disk, automatically creating any missing parent directories.
- `edit_file`: Performs exact string replacements. Fails deterministically if the target string is not unique, forcing the LLM to provide precise context.
- `glob`: Traverses the workspace to find files matching specific patterns, returning results sorted by modification time to prioritize active context.
- `grep`: Executes regular expression searches across the filesystem, returning matches with surrounding context lines.
- `list_directory`: Maps directory topologies, sorting directories first and including precise file sizes.
- `task`: Spawns autonomous sub-agents to handle scoped, multi-step objectives. Sub-agents inherit the parent's restricted capability bounds.
- `spark`: Reads and mutates the agent's identity manifest (`~/.astrid/spark.toml`), allowing the agent to dynamically evolve its own persona and operational parameters.

### The Tool Registry

The `ToolRegistry` serves as the central directory for these built-ins. During runtime initialization, it automatically extracts the `input_schema` and `description` from each tool, compiling them into a format ready for LLM consumption. Because these tools lack colons in their names (e.g., `read_file` instead of `filesystem:read_file`), the Astralis runtime instantly identifies them as local, in-process functions rather than remote MCP capabilities.

### Tool Context & State

Tools operate statelessly with one exception: they share a `ToolContext`. This context holds the current working directory, a reference to the sub-agent spawner, and the workspace root boundary. When the `bash` tool executes `cd /src`, the updated path is written back to the context, ensuring the next tool invocation starts from the correct location.

## Quick Start

*Note: Security constraints (like path confinement and user authorization) are not enforced at this layer. They are handled by `astrid-approval` prior to tool execution.*

## Development

To work on `astrid-tools` directly within the Astralis workspace:

```bash
# Run tests specifically for this crate
cargo test -p astrid-tools
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
