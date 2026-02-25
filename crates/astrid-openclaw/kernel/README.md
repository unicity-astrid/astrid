# QuickJS WASM Kernel

The `engine.wasm` file is the QuickJS engine compiled to `wasm32-wasip1`, used by the
astrid-openclaw compiler to embed JavaScript into WASM plugins via Wizer pre-initialization.

**This file is NOT checked into git.** It is built from source and placed here by
`scripts/build-quickjs-kernel.sh`. The `build.rs` generates a placeholder stub when the
kernel is absent — compilation succeeds but the compiler will error at runtime with
instructions to build the real kernel.

## Quick Start

```bash
./scripts/build-quickjs-kernel.sh
```

## Building Manually

The kernel is built from [extism/js-pdk](https://github.com/extism/js-pdk)'s `core` crate
(fork: [nicholasgasior/extism-js](https://github.com/nicholasgasior/extism-js) v1.6.0).

### Prerequisites

- Rust with `wasm32-wasip1` target: `rustup target add wasm32-wasip1`
- [wasi-sdk](https://github.com/WebAssembly/wasi-sdk) (required by `rquickjs` C bindings)
- Node.js + npm (for the JS prelude)
- Optional: `wasm-opt` from [Binaryen](https://github.com/WebAssembly/binaryen) for size optimization

### Build Steps

```bash
# Clone the js-pdk repository
git clone https://github.com/nicholasgasior/extism-js.git
cd extism-js

# Install wasi-sdk (sets CC/AR for wasm32-wasip1)
sh install-wasi-sdk.sh

# Build the JS prelude
cd crates/core/src/prelude
npm install
npm run build
cd ../../../..

# Build the core to wasm32-wasip1
cargo build --release --target=wasm32-wasip1

# Optional: optimize with wasm-opt
wasm-opt --enable-reference-types --enable-bulk-memory --strip -O3 \
  target/wasm32-wasip1/release/js_pdk_core.wasm \
  -o target/wasm32-wasip1/release/js_pdk_core.wasm

# Copy to this directory
cp target/wasm32-wasip1/release/js_pdk_core.wasm /path/to/astrid/crates/astrid-openclaw/kernel/engine.wasm
```

### Updating the Hash

After placing a new `engine.wasm`, update the blake3 hash:

```bash
cd crates/astrid-openclaw/kernel
b3sum engine.wasm > engine.wasm.blake3
```

## Hash Verification

The `build.rs` verifies the kernel's blake3 hash against `engine.wasm.blake3` at compile
time. If the hash doesn't match, the build fails with a clear error.
