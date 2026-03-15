# astrid-openclaw

[![Crates.io](https://img.shields.io/crates/v/astrid-openclaw)](https://crates.io/crates/astrid-openclaw)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The TypeScript-to-WASM compiler for the Astrid OS.**

Astrid capsules are WASM. OpenClaw plugins are TypeScript. This crate bridges the gap. It reads an `openclaw.plugin.json` directory, classifies the plugin as Tier 1 (pure WASM) or Tier 2 (sandboxed Node.js subprocess), and produces a ready-to-load Astrid capsule. The entire pipeline is pure Rust. No Node.js, no esbuild, no Rollup on the compilation host.

## Why it exists

The capsule model only works if there are capsules to run. OpenClaw has an existing ecosystem of tool plugins written in TypeScript and JavaScript. Rather than asking plugin authors to rewrite everything in Rust, this compiler absorbs the existing ecosystem into Astrid's sandbox with zero manual porting.

## Compilation pipeline

```text
Plugin.ts -> [OXC transpiler] -> Plugin.js -> [shim.rs] -> shimmed.js
  -> [Wizer + QuickJS kernel] -> raw.wasm -> [export stitcher] -> plugin.wasm
```

**Tier 1 (WASM).** Self-contained plugins compile all the way to WASM. OXC strips TypeScript types and converts ESM to CommonJS. The shim wraps the plugin code for QuickJS. Wizer pre-initializes a QuickJS engine with the shimmed source, snapshotting initialized state into the output WASM. A binary rewriting pass stitches in named Astrid ABI exports (`astrid_tool_call`, `describe-tools`, etc.) with zero external dependencies.

**Tier 2 (Node.js subprocess).** Plugins with npm dependencies, channels, providers, or unsupported Node.js imports cannot compile to pure WASM. These run as a sandboxed Node.js subprocess via an embedded MCP bridge. The compiler copies the source tree (skipping `node_modules`, `.git`, build artifacts), writes the bridge script, and generates an MCP-backed `Capsule.toml`.

Tier classification is automatic. The compiler inspects the manifest, `package.json`, and import graph.

## Key behaviors

- **BLAKE3-keyed compilation cache.** Compiled artifacts cached under `~/.astrid/cache/openclaw/` keyed by BLAKE3 hash of the shimmed source. Invalidates automatically when source, bridge version, or QuickJS kernel changes.
- **Config schema validation.** Plugin `configSchema` validated at both build time and activation time. Secret fields auto-detected by name heuristics (`apiKey`, `accessToken`, `clientSecret`).
- **Build isolation.** `build.rs` runs all subprocesses with a sanitized environment. Only an explicit allowlist of variables is forwarded.
- **Symlink rejection.** Symlinks in the plugin source tree are rejected outright. No traversal escapes.

## Quick start

```toml
[dependencies]
astrid-openclaw = "0.2"
```

```rust
use std::collections::HashMap;
use astrid_openclaw::pipeline::{compile_plugin, CompileOptions, default_cache_dir};

let opts = CompileOptions {
    plugin_dir: "/path/to/my-openclaw-plugin".as_ref(),
    output_dir: "/path/to/output".as_ref(),
    config: &HashMap::new(),
    cache_dir: default_cache_dir().as_deref(),
    js_only: false,
    no_cache: false,
};

let result = compile_plugin(&opts)?;
println!("compiled {} (tier: {})", result.astrid_id, result.tier);
```

## Development

```bash
cargo test -p astrid-openclaw
```

Most tests use `tempfile` and do not require a QuickJS kernel. End-to-end WASM compilation tests require the kernel binary (`crates/astrid-openclaw/kernel/engine.wasm`) to be built first.

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
