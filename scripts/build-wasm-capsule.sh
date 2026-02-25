#!/usr/bin/env bash
set -e

if [ -z "$1" ]; then
    echo "Usage: $0 <path-to-capsule-crate>"
    echo "Example: $0 crates/astrid-capsule-shell"
    exit 1
fi

CRATE_DIR=$1
if [ ! -f "$CRATE_DIR/Cargo.toml" ]; then
    echo "Error: No Cargo.toml found in $CRATE_DIR"
    exit 1
fi

if [ ! -f "$CRATE_DIR/Capsule.toml" ]; then
    echo "Error: No Capsule.toml found in $CRATE_DIR"
    exit 1
fi

# Extract the crate name
CRATE_NAME=$(grep -m 1 '^name = ' "$CRATE_DIR/Cargo.toml" | cut -d '"' -f 2)
# Convert hyphens to underscores for the wasm binary name
WASM_NAME=$(echo "$CRATE_NAME" | tr '-' '_')

echo "🔨 Building WASM capsule: $CRATE_NAME..."
cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --target wasm32-wasip1 --release

# Setup the distribution package folder
DIST_DIR="$CRATE_DIR/dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

echo "📦 Packaging capsule into $DIST_DIR..."

# Copy the compiled WASM binary
cp "$CRATE_DIR/target/wasm32-wasip1/release/${WASM_NAME}.wasm" "$DIST_DIR/"

# Copy the manifest
cp "$CRATE_DIR/Capsule.toml" "$DIST_DIR/"

# Update the entrypoint in the distributed manifest to point to the local WASM file
# This ensures it works cleanly when the user installs the distributed folder
sed -i.bak "s|target/wasm32-wasip1/release/${WASM_NAME}.wasm|${WASM_NAME}.wasm|g" "$DIST_DIR/Capsule.toml"
rm -f "$DIST_DIR/Capsule.toml.bak"

echo "✅ Capsule packed successfully!"
echo "You can now install it using: astrid capsule install $DIST_DIR"
