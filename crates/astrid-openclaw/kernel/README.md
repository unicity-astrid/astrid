# astrid-openclaw kernel

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The QuickJS WASM engine that powers Tier 1 plugin compilation.**

This directory holds `engine.wasm`, a QuickJS build targeting `wasm32-wasip1`. The `astrid-openclaw` crate embeds it via `include_bytes!` and feeds it to Wizer for pre-initialization. Without this file, Tier 1 compilation fails at runtime with build instructions.

## Why it exists

Tier 1 plugins compile TypeScript into a WASM module by pre-initializing a QuickJS engine with the plugin source. That engine is this file. It is compiled from source, not checked into git. The build produces a single `engine.wasm` plus a `engine.wasm.blake3` hash that `build.rs` verifies at compile time. Hash mismatch fails the build. This prevents silent use of a stale or tampered engine.

When the engine is absent, `build.rs` generates a placeholder stub so workspace compilation succeeds. The stub errors at runtime, not compile time, pointing the developer at the build command.

## Building

```bash
# Option 1: manual build script (requires wasi-sdk + wasm32-wasip1 target)
./scripts/build-quickjs-kernel.sh

# Option 2: auto-build during cargo build
ASTRID_AUTO_BUILD_KERNEL=1 cargo build -p astrid-openclaw
```

After placing a new `engine.wasm`, regenerate the hash:

```bash
cd crates/astrid-openclaw/kernel
b3sum engine.wasm > engine.wasm.blake3
```

## Development

```bash
cargo test -p astrid-openclaw
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../../LICENSE-MIT) and [LICENSE-APACHE](../../../LICENSE-APACHE).
