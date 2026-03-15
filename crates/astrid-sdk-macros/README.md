# astrid-sdk-macros

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The compiler that turns a Rust impl block into a WASM capsule.**

Capsules are processes in the Astrid OS model. A process needs a defined ABI: exported entry points the kernel can call, a dispatch table for routing tool/command/hook/cron invocations, JSON Schema generation for discovery, and lifecycle hooks for install and upgrade. Writing this by hand means hundreds of lines of `extern "C"` boilerplate per capsule.

`#[capsule]` eliminates all of it. One attribute on an `impl` block, and the macro generates six WASM exports (`astrid_tool_call`, `astrid_command_run`, `astrid_hook_trigger`, `astrid_cron_trigger`, `astrid_export_schemas`, `run`) plus conditional `astrid_install` and `astrid_upgrade` exports when the corresponding lifecycle attributes are present.

Do not depend on this crate directly. Use the re-export from `astrid-sdk`.

## What it generates

For a capsule with two tools, an install hook, and a cron handler, the macro emits:

- **`astrid_tool_call`** dispatches by tool name, deserializes the JSON args into the method's parameter type, calls the method, serializes the result back
- **`astrid_command_run`** same pattern for slash commands
- **`astrid_hook_trigger`** same pattern for interceptors
- **`astrid_cron_trigger`** same pattern for cron handlers
- **`astrid_export_schemas`** returns JSON Schema for every tool/command/interceptor/cron, generated via `schemars`
- **`astrid_install`** / **`astrid_upgrade`** lifecycle entry points, only emitted when `#[astrid::install]` / `#[astrid::upgrade]` attributes are present
- **`run`** long-lived event loop export, only emitted when `#[astrid::run]` is present

Errors propagate as kernel-visible strings. Panics are caught by the Extism runtime and surfaced as WASM traps.

## Attribute reference

| Attribute | Signature | Notes |
|---|---|---|
| `#[astrid::tool("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | Args param optional for parameterless tools |
| `#[astrid::command("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | Same as tool |
| `#[astrid::interceptor("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | Same as tool |
| `#[astrid::cron("name")]` | `fn(&self, args: T) -> Result<U, SysError>` | Same as tool |
| `#[astrid::install]` | `fn(&self) -> Result<(), SysError>` | No arguments. Compile error if args present |
| `#[astrid::upgrade]` | `fn(&self, prev_version: &str) -> Result<(), SysError>` | Must be `&str`, not `String`. Compile error otherwise |
| `#[astrid::run]` | `fn(&self) -> Result<(), SysError>` | Long-lived event loop. No arguments |
| `#[astrid::mutable]` | (modifier) | Valid only on tool/command/interceptor/cron. Embeds `"mutable": true` in schema |

Name arguments on tool/command/interceptor/cron are optional. Omit the string and the macro uses the Rust method name.

## Compile-time enforcement

The macro rejects invalid capsule definitions at compile time, not runtime:

- Duplicate `#[astrid::install]` or `#[astrid::upgrade]` attributes produce `compile_error!`
- `#[astrid::upgrade]` with `String` instead of `&str` produces `compile_error!`
- `#[astrid::install]` with any arguments produces `compile_error!`
- `#[astrid::mutable]` on lifecycle hooks produces `compile_error!`

## Stateful vs stateless

`#[capsule]` creates a `OnceLock`-backed singleton. State lives in KV or in-memory and the capsule manages it explicitly.

`#[capsule(state)]` auto-loads the struct from KV key `"__state"` before each call and auto-saves after. The struct must implement `Serialize + DeserializeOwned + Default`.

## Development

```bash
cargo test -p astrid-sdk-macros
```

The test suite (1,300+ lines) covers schema generation, lifecycle export presence/absence, compile-error detection, stateful round-trips, and mutable flag propagation.

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
