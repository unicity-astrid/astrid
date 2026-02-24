#!/usr/bin/env bash
# Compile the test-plugin-guest Rust crate to WASM and install as a test fixture.
#
# Prerequisites:
#   rustup target add wasm32-unknown-unknown
#
# Usage:
#   ./scripts/compile-test-plugin.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

GUEST_CRATE="$ROOT_DIR/crates/test-plugin-guest"
FIXTURE_DIR="$ROOT_DIR/crates/astrid-integration-tests/tests/fixtures"

# Check target is installed
if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
    echo "Installing wasm32-unknown-unknown target..."
    rustup target add wasm32-unknown-unknown
fi

echo "==> Building test-plugin-guest..."
cd "$GUEST_CRATE"
cargo build --release

echo "==> Copying fixture..."
mkdir -p "$FIXTURE_DIR"
cp "$GUEST_CRATE/target/wasm32-unknown-unknown/release/test_plugin_guest.wasm" \
   "$FIXTURE_DIR/test-all-endpoints.wasm"

SIZE=$(wc -c < "$FIXTURE_DIR/test-all-endpoints.wasm" | tr -d ' ')
echo "==> Success: $FIXTURE_DIR/test-all-endpoints.wasm ($SIZE bytes)"
echo ""
echo "Run the integration tests:"
echo "  cargo test -p astrid-integration-tests --test wasm_e2e"
