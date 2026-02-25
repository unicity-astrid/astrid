# astrid-sdk-macros

[![Crates.io](https://img.shields.io/crates/v/astrid-sdk-macros)](https://crates.io/crates/astrid-sdk-macros)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Procedural macros for the Astrid OS System SDK, providing the zero-boilerplate boundary between your Rust code and the Astrid OS kernel.

This crate is a core component of the Astralis workspace. It supplies the `#[capsule]` attribute macro, which automatically generates the required WebAssembly (`extern "C"`) exports, implements the Astrid OS Inbound ABI, and handles seamless JSON serialization across the host-guest boundary.

## Core Features

* **Declarative WASM Entrypoints:** Convert standard Rust `impl` blocks into Extism PDK plugins with a single attribute.
* **Automated ABI Implementation:** Generates the precise `extern "C"` functions expected by the Astrid Kernel (`astrid_tool_call`, `astrid_command_run`, etc.).
* **Zero-Boilerplate Serialization:** Automatically deserializes inbound JSON payloads into Rust types and serializes results back across the boundary.
* **Transparent State Management:** Supports seamless persistence of capsule state using the Astrid OS Key-Value store across stateless WebAssembly invocations.

## Architecture

The `#[capsule]` macro acts as an ABI bridge. It parses your `impl` block and generates `#[extism_pdk::plugin_fn]` exports that strictly conform to the Astrid OS Inbound ABI.

For example, annotating a method with `#[astrid::tool("foo")]` generates an `astrid_tool_call` Extism plugin function. When the Astrid kernel invokes this function with a JSON payload, the generated code performs the following pipeline:

1. Deserializes the kernel's boundary type (`__AstridToolRequest`).
2. Matches the request name against your defined routes.
3. Deserializes the internal arguments byte array into your method's specific Rust type.
4. Invokes your method.
5. Serializes the `Result` and returns the bytes to the Extism host environment.

This entirely abstracts away the complexities of WebAssembly memory management, pointer passing, and Extism PDK compliance, allowing developers to write idiomatic Rust functions that securely execute within the Astrid OS sandbox.

## Quick Start

This crate is an internal dependency of the Astralis SDK. You should not depend on it directly. Instead, use the re-exports provided by `astrid-sdk`:

```toml
[dependencies]
astrid-sdk = { workspace = true }
```

The primary entry point is the `#[capsule]` attribute, applied to an `impl` block for a struct that implements `std::default::Default`.

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
    fn ping(&self, _args: ()) -> Result<String, SysError> {
        Ok("pong".to_string())
    }
}
```

### Stateful Capsules

WebAssembly execution in Astrid OS is fundamentally stateless. To maintain state between invocations, you can opt into automatic state persistence by passing the `state` argument:

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
    fn increment(&mut self, _args: ()) -> Result<i32, SysError> {
        self.count += 1;
        Ok(self.count)
    }
}
```

When `#[capsule(state)]` is used, the generated WASM exports automatically:
1. Load the struct instance from the Astrid KV store (under the key `"__state"`) before execution, falling back to `Default` if empty.
2. Execute the requested method.
3. Serialize and save the mutated struct back to the KV store before returning execution to the kernel.

### Routing Attributes

Inside the `#[capsule]` block, individual methods are exposed to the kernel using routing attributes. The string argument defines the specific name the kernel will use to route requests to that method.

* **`#[astrid::tool("name")]`**: Exposes the method to the LLM Agent via the OS Event Bus. Maps to the `astrid_tool_call` ABI export.
* **`#[astrid::command("name")]`**: Exposes the method to human users via Uplink slash-commands (e.g., CLI, Telegram). Maps to the `astrid_command_run` ABI export.
* **`#[astrid::interceptor("name")]`**: Executed synchronously by the Kernel during OS lifecycle events. Maps to the `astrid_hook_trigger` ABI export.
* **`#[astrid::cron("name")]`**: Executed by the Kernel's scheduler for time-based tasks. Maps to the `astrid_cron_trigger` ABI export.

## Development

```bash
cargo test -p astrid-sdk-macros
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
