# astrid-openclaw

[![Crates.io](https://img.shields.io/crates/v/astrid-openclaw)](https://crates.io/crates/astrid-openclaw)
[![docs.rs](https://img.shields.io/docsrs/astrid-openclaw)](https://docs.rs/astrid-openclaw)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)

Converts OpenClaw tool plugins into Astrid WASM plugins.

**astrid-openclaw** is the compatibility bridge that brings OpenClaw's plugin ecosystem into the Astrid runtime. It reads an `openclaw.plugin.json` directory, detects whether the plugin can run inside the WASM sandbox or needs a full Node.js process, and produces a ready-to-load Astrid capsule: a compiled `plugin.wasm` and a `Capsule.toml` manifest. The entire pipeline is pure Rust - no external build tools, no Node.js installed on the compilation host, no esbuild or Rollup.

## Core Features

- **Two-tier runtime detection.** Automatically classifies plugins: self-contained single-file plugins compile to WASM (Tier 1); plugins with npm dependencies, channels, providers, or unsupported Node.js imports run as a sandboxed Node.js subprocess via an embedded MCP bridge (Tier 2).
- **Pure-Rust TS/JS transpilation.** Uses [OXC](https://oxc.rs) to parse TypeScript or JavaScript, strip types, and post-process ESM to CommonJS. No external transpiler required.
- **Embedded QuickJS kernel.** The pre-built QuickJS WASM engine (`wasm32-wasip1`) is compiled directly into the binary via `include_bytes!`. Wizer pre-initializes it with the plugin JS at compile time, snapshotting the initialized state into the output WASM.
- **Subprocess Wizer isolation.** Wizer runs in a hidden child process so the plugin JS can be piped to the kernel's WASI stdin without interfering with the parent process environment.
- **Custom WASM export stitching.** After Wizer produces a pre-initialized module, a targeted binary rewriting pass adds the named Astrid ABI exports (`astrid_tool_call`, `describe-tools`, etc.) by appending wrapper functions that delegate to the kernel's `__invoke_i32` export. Replaces `wasm-merge` with zero external dependencies.
- **Blake3-keyed compilation cache.** Compiled WASM artifacts are cached under `~/.astrid/cache/openclaw/` keyed by the blake3 hash of the shimmed source. Cache entries are invalidated automatically when the source, bridge version, or QuickJS kernel changes. Writes are atomic (temp dir + rename).
- **Config schema validation.** Plugin `configSchema` is validated at both build time (unknown keys rejected) and activation time (required keys enforced). Secret fields are auto-detected by name heuristics (`apiKey`, `accessToken`, `clientSecret`, etc.) and written as `type = "secret"` in `Capsule.toml`.
- **Security-first build isolation.** `build.rs` runs all subprocesses with a sanitized environment - only an explicit allowlist of variables is forwarded - to prevent CI secrets and Cargo state from leaking into untrusted build steps.

## How It Works

```text
Plugin.ts
  -> [OXC transpiler]  Strip TS types, ESM -> CJS
  -> [shim.rs]         Wrap in IIFE, inject host function stubs, defer activation
  -> [Wizer + QuickJS] Pre-initialize JS engine, snapshot WASM linear memory
  -> [export_stitch]   Stitch named Astrid ABI exports into the WASM binary
  -> plugin.wasm + Capsule.toml
```

All four stages run inside the calling process (or a short-lived subprocess for Wizer). No files are left in a partially-written state.

### Tier 1 vs Tier 2

Tier detection is automatic and runs before compilation:

| Signal | Tier |
|---|---|
| Manifest declares `channels` or `providers` | Node (Tier 2) |
| `package.json` has non-empty `dependencies` | Node (Tier 2) |
| Entry point imports `node:http`, `node:net`, etc. | Node (Tier 2) |
| Entry point has local relative imports (`./`, `../`) | Node (Tier 2) |
| Everything else | WASM (Tier 1) |

Tier 2 output copies the plugin source into the output directory, writes the embedded `astrid_bridge.mjs` MCP bridge script, and generates a `Capsule.toml` with an `[[mcp_server]]` section instead of a WASM component entry.

## Quick Start

Add to `Cargo.toml`:

```toml
[dependencies]
astrid-openclaw = "0.2"
```

Compile a plugin directory:

```rust
use std::collections::HashMap;
use astrid_openclaw::pipeline::{compile_plugin, CompileOptions};
use astrid_openclaw::pipeline::default_cache_dir;

let opts = CompileOptions {
    plugin_dir: "/path/to/my-openclaw-plugin".as_ref(),
    output_dir:  "/path/to/output".as_ref(),
    config:      &HashMap::new(),
    cache_dir:   default_cache_dir().as_deref(),
    js_only:     false,
    no_cache:    false,
};

let result = compile_plugin(&opts)?;
println!("compiled {} (tier: {})", result.astrid_id, result.tier);
```

The output directory will contain:

- `plugin.wasm` - compiled, pre-initialized WASM module (Tier 1 only)
- `shim.js` - intermediate JavaScript shim, useful for debugging (Tier 1 only)
- `Capsule.toml` - Astrid capsule manifest
- `astrid_bridge.mjs` + plugin source (Tier 2 only)

## API Reference

### Key Types

**`pipeline::CompileOptions<'a>`** - Input to the top-level compilation function.

| Field | Type | Description |
|---|---|---|
| `plugin_dir` | `&Path` | Directory containing `openclaw.plugin.json` |
| `output_dir` | `&Path` | Where compiled artifacts are written |
| `config` | `&HashMap<String, Value>` | Config key-value pairs validated against `configSchema` |
| `cache_dir` | `Option<&Path>` | Root of the compilation cache; `None` disables caching |
| `js_only` | `bool` | Skip WASM compilation, emit only `shim.js` |
| `no_cache` | `bool` | Bypass cache even if `cache_dir` is set |

**`pipeline::CompileResult`** - Returned on success.

| Field | Type | Description |
|---|---|---|
| `astrid_id` | `String` | Normalized plugin ID (lowercase, hyphens) |
| `tier` | `PluginTier` | `Wasm` or `Node` |
| `manifest` | `OpenClawManifest` | Parsed source manifest |
| `cached` | `bool` | Whether the result was served from cache (Tier 1 only) |

**`tier::PluginTier`** - `Wasm` or `Node`. Implements `Display` (`"wasm"`, `"node"`).

**`manifest::OpenClawManifest`** - Parsed `openclaw.plugin.json`. Key fields: `id`, `config_schema`, `name`, `version`, `description`, `kind`, `channels`, `providers`, `skills`, `ui_hints`.

**`manifest::convert_id(openclaw_id: &str) -> BridgeResult<String>`** - Normalizes an OpenClaw ID to Astrid format: lowercased, underscores and dots replaced with hyphens, consecutive hyphens collapsed, leading/trailing hyphens stripped.

**`error::BridgeError`** - All pipeline errors. Variants: `Manifest`, `InvalidId`, `EntryPointNotFound`, `TranspileFailed`, `UnresolvedImports`, `CompileFailed`, `ExportStitchFailed`, `Cache`, `Output`, `ConfigValidation`, `Io`.

**`pipeline::default_cache_dir() -> Option<PathBuf>`** - Resolves `~/.astrid/cache/openclaw/`.

### Entry Point Resolution

The plugin entry point is resolved in this order:

1. `package.json` -> `openclaw.extensions[0]`
2. `src/index.ts`
3. `src/index.js`
4. `index.ts`
5. `index.js`

Entry points are validated against path traversal, absolute paths, and characters that could cause TOML or CLI injection.

## Building the QuickJS Kernel

Tier 1 WASM compilation requires the pre-built QuickJS kernel (`kernel/engine.wasm`). Without it the crate compiles but `compile_plugin` returns an error for Tier 1 plugins.

**Option 1 - manual build script** (requires `wasi-sdk` and `wasm32-wasip1` Rust target):

```bash
./scripts/build-quickjs-kernel.sh
```

**Option 2 - auto-build during `cargo build`:**

```bash
ASTRID_AUTO_BUILD_KERNEL=1 cargo build -p astrid-openclaw
```

Auto-build clones [extism/js-pdk](https://github.com/extism/js-pdk) at `v1.6.0`, installs wasi-sdk, and compiles the QuickJS core to `wasm32-wasip1`. For supply-chain integrity, pin the resulting blake3 hash by setting `EXPECTED_KERNEL_HASH` in `build.rs` and use `ASTRID_REQUIRE_KERNEL_HASH=1` in CI.

## Development

Run the unit tests:

```bash
cargo test -p astrid-openclaw
```

Most tests use `tempfile` to create isolated plugin directories and do not require a QuickJS kernel - the transpiler, manifest parser, tier detector, cache, shim generator, export stitcher, and output manifest tests all run without it. End-to-end WASM compilation tests (in `tests/`) require the kernel and the `astrid` CLI binary to be built first.

## Project Structure

| Path | Purpose |
|---|---|
| `src/pipeline.rs` | Top-level `compile_plugin` entry point; orchestrates all stages |
| `src/tier.rs` | Tier detection logic (Wasm vs Node) |
| `src/manifest.rs` | `openclaw.plugin.json` parsing, ID conversion, env field extraction |
| `src/transpiler.rs` | OXC-based TS/JS parse, type strip, ESM-to-CJS conversion |
| `src/shim.rs` | JS shim generation wrapping plugin code in the Astrid host ABI |
| `src/compiler.rs` | Wizer subprocess runner; embeds and invokes the QuickJS kernel |
| `src/export_stitch.rs` | WASM binary rewriter adding named Astrid ABI exports |
| `src/cache.rs` | Blake3-keyed compilation artifact cache |
| `src/output.rs` | `Capsule.toml` manifest generation for Tier 1 |
| `src/node_bridge.rs` | Embeds and writes `astrid_bridge.mjs` for Tier 2 |
| `build.rs` | Kernel embedding, hash verification, optional auto-build |
| `bridge/astrid_bridge.mjs` | Universal MCP bridge script for Tier 2 plugins |
| `kernel/` | Pre-built QuickJS `engine.wasm` and its blake3 hash file |

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
