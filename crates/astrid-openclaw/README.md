# astrid-openclaw

[![Crates.io](https://img.shields.io/crates/v/astrid-openclaw)](https://crates.io/crates/astrid-openclaw)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The OpenClaw-to-WASM compilation pipeline for the Astralis OS workspace.

**astrid-openclaw** translates single-file OpenClaw tool plugins (written in TypeScript or JavaScript) into native Astrid WASM plugins. By compiling logic into a WebAssembly module, it allows pure-logic OpenClaw tools to execute directly inside the secure, zero-trust Astrid WASM sandbox without spinning up an external Node.js process. 

This crate implements the Tier 1 compatibility path for OpenClaw. It runs as a self-contained, pure-Rust pipeline — no external build tools, Node.js installations, or CLI dependencies required.

## Core Features

- **Pure-Rust Transpilation**: Leverages OXC to parse, strip TypeScript types, and transpile source code to CommonJS at extreme speeds without esbuild or Rollup.
- **Embedded Engine Kernel**: Packages a pre-built QuickJS WebAssembly kernel (compiled to `wasm32-wasip1`) directly into the binary.
- **Subprocess Wizer Initialization**: Uses an embedded Wizer configuration to pre-initialize the QuickJS environment and snapshot the plugin state at compile time, reducing runtime startup latency to near zero.
- **Direct WASM Export Stitching**: Replaces Binaryen's `wasm-merge` with a custom, zero-overhead WASM transformation pass that wires OpenClaw functions to Phase 4 Inbound ABI exports (`astrid_tool_call`, `astrid_hook_trigger`).
- **Transparent Syscall Mapping**: Generates a runtime shim that polyfills Node.js core modules (`node:fs`, `node:path`, `node:os`) and the OpenClaw context, routing operations directly to `astrid::sys` host functions.

## Architecture

The conversion process executes in four distinct, pure-Rust phases:

1. **Transpilation (`transpiler.rs`)**: Reads the plugin's entry point, validates that only allowed imports are present (rejecting runtime external dependencies), strips TypeScript types via OXC, and performs an ESM-to-CJS conversion.
2. **Shimming (`shim.rs`)**: Wraps the transpiled code in an Immediately Invoked Function Expression (IIFE). It injects deferred host function wrappers and an OpenClaw context mock. All host function calls are deferred until `_ensureActivated()` runs upon the first export invocation, preventing Wizer initialization failures during the snapshot phase.
3. **Snapshotting (`compiler.rs`)**: Spawns a hidden child process (`wizer-internal` subcommand) to pipe the shimmed JavaScript into the embedded QuickJS kernel. Wizer executes the kernel, evaluating the JavaScript, and snapshots the entire WebAssembly linear memory into a pre-initialized module.
4. **Stitching (`export_stitch.rs`)**: Reads the Wizer-produced WebAssembly module and directly manipulates the function, type, and export sections. It adds named exports (e.g., `describe-tools`, `astrid_tool_call`) that delegate to the QuickJS kernel's `__invoke_i32(index)` export based on the alphabetical order of the `module.exports` keys.

### Tier 1 vs Tier 2 Execution

`astrid-openclaw` powers the **Tier 1** execution path for Astralis. Because QuickJS within a WASM sandbox lacks an asynchronous event loop and network stack, plugins requiring async I/O, `fetch()`, or complex Node.js dependencies cannot be compiled to Tier 1 WASM. 

For those plugins, Astralis automatically falls back to the **Tier 2 MCP Bridge** (`packages/openclaw-mcp-bridge/`). This bridge runs the plugin in a native Node.js process and interfaces with Astralis via the Model Context Protocol over stdio.

## Quick Start

Convert an existing OpenClaw plugin directory containing an `openclaw.plugin.json` manifest:

```bash
# From the astralis workspace root
cargo run -p astrid-openclaw -- convert 
    --plugin-dir /path/to/my-openclaw-plugin 
    --output ./output 
    --config '{"apiKey": "test_key"}'
```

The output directory will contain:
- `plugin.wasm`: The compiled, pre-initialized WebAssembly module ready for the Astrid runtime.
- `shim.js`: The intermediate JavaScript shim (useful for debugging).
- `plugin.toml`: The translated Astrid manifest.

## Development

To rebuild the embedded QuickJS kernel (requires `wasi-sdk` and the `wasm32-wasip1` Rust target):

```bash
# Execute from the workspace root
./scripts/build-quickjs-kernel.sh
```

To run tests for the compiler and transpiler:

```bash
cargo test -p astrid-openclaw
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
