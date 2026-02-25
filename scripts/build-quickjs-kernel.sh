#!/usr/bin/env bash
# Build the QuickJS WASM kernel from extism/js-pdk and install it.
#
# This script clones the js-pdk repository, builds the QuickJS core crate
# to wasm32-wasip1, and copies the resulting WASM to the kernel directory.
#
# Prerequisites:
#   - Rust with wasm32-wasip1 target: rustup target add wasm32-wasip1
#   - wasi-sdk (for rquickjs C bindings): https://github.com/WebAssembly/wasi-sdk
#   - Node.js + npm (for the JS prelude)
#   - Optional: wasm-opt from Binaryen (for size optimization)
#
# Usage:
#   ./scripts/build-quickjs-kernel.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
KERNEL_DIR="$ROOT_DIR/crates/astrid-openclaw/kernel"
BUILD_DIR="${TMPDIR:-/tmp}/quickjs-kernel-build"

JS_PDK_REPO="https://github.com/nicholasgasior/extism-js.git"
JS_PDK_TAG="v1.6.0"

# Check wasm32-wasip1 target is installed
if ! rustup target list --installed | grep -q wasm32-wasip1; then
    echo "Installing wasm32-wasip1 target..."
    rustup target add wasm32-wasip1
fi

echo "==> Cloning js-pdk ${JS_PDK_TAG}..."
rm -rf "$BUILD_DIR"
git clone --depth 1 --branch "$JS_PDK_TAG" "$JS_PDK_REPO" "$BUILD_DIR"

echo "==> Installing wasi-sdk..."
cd "$BUILD_DIR"
sh install-wasi-sdk.sh

echo "==> Building JS prelude..."
cd "$BUILD_DIR/crates/core/src/prelude"
npm install
npm run build

echo "==> Building QuickJS core (wasm32-wasip1)..."
cd "$BUILD_DIR"
cargo build --release --target=wasm32-wasip1

BUILT_WASM="$BUILD_DIR/target/wasm32-wasip1/release/js_pdk_core.wasm"
if [ ! -f "$BUILT_WASM" ]; then
    echo "ERROR: Build output not found at $BUILT_WASM"
    exit 1
fi

# Optional: optimize with wasm-opt
if command -v wasm-opt &>/dev/null; then
    echo "==> Optimizing with wasm-opt..."
    wasm-opt --enable-reference-types --enable-bulk-memory --strip -O3 \
        "$BUILT_WASM" -o "$BUILT_WASM"
fi

echo "==> Installing kernel..."
mkdir -p "$KERNEL_DIR"
cp "$BUILT_WASM" "$KERNEL_DIR/engine.wasm"

# Update blake3 hash
if command -v b3sum &>/dev/null; then
    cd "$KERNEL_DIR"
    b3sum engine.wasm > engine.wasm.blake3
    echo "==> Updated blake3 hash"
else
    echo "==> WARNING: b3sum not found, skipping hash update"
    echo "    Install with: cargo install b3sum"
fi

SIZE=$(wc -c < "$KERNEL_DIR/engine.wasm" | tr -d ' ')
echo "==> Success: $KERNEL_DIR/engine.wasm ($SIZE bytes)"
echo ""
echo "Rebuild astrid-openclaw to embed the kernel:"
echo "  cargo build -p astrid-openclaw"

# Cleanup
rm -rf "$BUILD_DIR"
