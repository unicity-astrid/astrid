# astrid-sdk-macros

[![Crates.io](https://img.shields.io/crates/v/astrid-sdk-macros)](https://crates.io/crates/astrid-sdk-macros)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)

Procedural macros for the Astrid OS System SDK — the zero-boilerplate bridge between idiomatic Rust and the Astrid Kernel's WebAssembly ABI.

This crate provides the `#[capsule]` attribute macro, which transforms a standard Rust `impl` block into a fully compliant Extism PDK plugin. It generates all required `extern "C"` WebAssembly exports (`astrid_tool_call`, `astrid_command_run`, `astrid_hook_trigger`, `astrid_cron_trigger`, `astrid_export_schemas`, `astrid_install`, `astrid_upgrade`, `run`), implements the Astrid OS Inbound ABI, and handles JSON serialization across the host-guest boundary — without any manual FFI or memory management.

## Core Features

- **Single-attribute ABI implementation.** One `#[capsule]` on an `impl` block generates every WASM export the kernel expects. No manual `#[no_mangle]` functions, no manual Extism PDK boilerplate.
- **Declarative method routing.** Individual methods are wired to specific dispatch tables using inner attributes. The routing name and method name are independent; the name can also be inferred from the function name when no string argument is provided.
- **Automatic JSON serialization.** Inbound kernel payloads are deserialized into the method's argument type via `serde_json`. Results are serialized back before returning to the host. Serialization errors propagate as kernel-visible error strings, not panics.
- **Opt-in stateful persistence.** Pass `#[capsule(state)]` to enable automatic load-from-KV / save-to-KV around every dispatch, keyed under `"__state"`. Stateless capsules use a `OnceLock`-backed singleton instead.
- **Per-tool mutability annotation.** Mark a tool `#[astrid::mutable]` to embed `"mutable": true` in its generated JSON Schema. Non-annotated tools get `"mutable": false`. The kernel uses this to gate approval workflows.
- **Auto-generated JSON Schema export.** The macro generates an `astrid_export_schemas` ABI function that returns a `BTreeMap<String, RootSchema>` for every registered tool, allowing CLI builders and the kernel to introspect argument shapes at runtime without running the capsule.
- **Lifecycle hooks.** Optional `#[astrid::install]` and `#[astrid::upgrade]` attributes generate dedicated ABI exports for first-install and version-upgrade events, with enforced signature contracts checked at compile time.
- **Long-lived run loops.** `#[astrid::run]` generates a `run` export for event-driven capsules. For stateful capsules, state is loaded at startup but deliberately not auto-saved — run loops are expected to be long-lived and manage their own persistence.
- **Compile-time contract enforcement.** Duplicate lifecycle hooks, wrong argument types on `#[astrid::upgrade]` (`String` instead of `&str`), args on `#[astrid::install]` or `#[astrid::run]`, and `#[astrid::mutable]` on lifecycle hooks all produce `compile_error!` at macro expansion time, not at link time or runtime.

## Quick Start

This crate is an internal dependency of `astrid-sdk`. Do not depend on it directly. Use the re-exports from `astrid-sdk`:

```toml
[dependencies]
astrid-sdk = { workspace = true }
```

## Usage

### Stateless capsule

The struct must implement `Default`. For stateless capsules the macro creates a `OnceLock`-backed singleton so you never call the constructor manually.

```rust
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SumArgs {
    pub a: i32,
    pub b: i32,
}

#[derive(Default)]
pub struct MyCapsule;

#[capsule]
impl MyCapsule {
    #[astrid::tool("calculate_sum")]
    fn calculate_sum(&self, args: SumArgs) -> Result<i32, SysError> {
        Ok(args.a + args.b)
    }

    #[astrid::command("ping")]
    fn ping(&self) -> Result<String, SysError> {
        Ok("pong".to_string())
    }
}
```

### Stateful capsule

Add `state` to the macro argument. The struct must also implement `Serialize` and `Deserialize` so the macro can round-trip it through the Astrid KV store.

```rust
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct CounterCapsule {
    count: i32,
}

#[capsule(state)]
impl CounterCapsule {
    #[astrid::tool("increment")]
    #[astrid::mutable]
    fn increment(&mut self, _args: ()) -> Result<i32, SysError> {
        self.count += 1;
        Ok(self.count)
    }

    #[astrid::tool("get_count")]
    fn get_count(&self, _args: ()) -> Result<i32, SysError> {
        Ok(self.count)
    }
}
```

State is loaded from KV key `"__state"` before dispatch and saved back after. A `JsonError` (key not found or corrupt bytes) falls back to `Default::default()`. A `HostError` propagates hard — the macro deliberately avoids silently resetting state on infrastructure failures.

### Lifecycle hooks

```rust
#[capsule(state)]
impl MyCapsule {
    /// Called once when the capsule is first installed.
    /// Must have signature: fn(&self) -> Result<(), SysError>
    #[astrid::install]
    fn install(&self) -> Result<(), SysError> {
        // seed initial state here
        Ok(())
    }

    /// Called when upgrading from a previous version.
    /// Must have signature: fn(&self, prev_version: &str) -> Result<(), SysError>
    #[astrid::upgrade]
    fn upgrade(&self, prev_version: &str) -> Result<(), SysError> {
        // migrate data from prev_version
        Ok(())
    }
}
```

### Run loop

```rust
#[capsule]
impl MyCapsule {
    /// Long-lived event-driven run loop.
    /// Must have signature: fn(&self) -> Result<(), SysError>
    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        loop {
            // block on IPC, handle events
        }
    }
}
```

## API Reference

### The `#[capsule]` macro

`#[proc_macro_attribute]` applied to an `impl` block. Accepts an optional `state` argument.

**Generated ABI exports** (always present):

| Export | Dispatch attribute | Description |
|---|---|---|
| `astrid_tool_call` | `#[astrid::tool("name")]` | LLM agent tool calls via the OS Event Bus |
| `astrid_command_run` | `#[astrid::command("name")]` | Human slash-commands via Uplink frontends |
| `astrid_hook_trigger` | `#[astrid::interceptor("name")]` | Synchronous kernel lifecycle interceptors |
| `astrid_cron_trigger` | `#[astrid::cron("name")]` | Scheduler-triggered time-based jobs |
| `astrid_export_schemas` | (automatic) | JSON Schema map for all registered tools |

**Generated ABI exports** (conditional):

| Export | Trigger attribute |
|---|---|
| `astrid_install` | `#[astrid::install]` present |
| `astrid_upgrade` | `#[astrid::upgrade]` present |
| `run` | `#[astrid::run]` present |

### Method attribute contracts enforced at compile time

| Attribute | Required signature | Notes |
|---|---|---|
| `#[astrid::tool("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | `args` may be omitted for parameterless tools |
| `#[astrid::command("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | Same as tool |
| `#[astrid::interceptor("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | Same as tool |
| `#[astrid::cron("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | Same as tool |
| `#[astrid::install]` | `fn(&self) -> Result<(), SysError>` | No arguments. Compile error if args present. |
| `#[astrid::upgrade]` | `fn(&self, prev_version: &str) -> Result<(), SysError>` | Must be `&str`, not `String`. |
| `#[astrid::run]` | `fn(&self) -> Result<(), SysError>` | No arguments. |
| `#[astrid::mutable]` | (modifier, no signature change) | Valid only on tool/command/interceptor/cron. |

The `name` string argument is optional for dispatch attributes. When omitted, the Rust method name is used as the routing key.

## Development

```bash
cargo test -p astrid-sdk-macros
```

The test suite exercises the `capsule_impl` function directly via `proc_macro2::TokenStream`, covering mutable schema generation, stateful KV round-trips, lifecycle hook exports, compile-error paths, and run-loop state behavior.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
